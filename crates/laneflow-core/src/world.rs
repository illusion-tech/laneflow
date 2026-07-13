//! Core world 与 fixed-step orchestration。

use indexmap::IndexMap;

use crate::{
    error::CoreError,
    event::{CoreEvent, VehicleChangedEdgeEvent, VehicleCompletedRouteEvent},
    graph::{EDGE_BOUNDARY_EPSILON, LaneGraph},
    handle::{EdgeHandle, RouteHandle, VehicleHandle, VehicleProfileHandle},
    id::validate_external_id,
    occupancy::{LeaderObservation, OccupancyScratch, Occupant},
    profile::{GEOMETRY_GAP_EPSILON, VehicleProfile, VehicleProfileRegistry},
    route::{Route, RouteRemoveRecord},
    time::{StepResult, TickInput},
    traffic::{InitialTrafficData, resolve_route_edges},
    vehicle::{
        Acceleration, EdgeProgress, Speed, VehicleDespawnRecord, VehicleSpawnInput, VehicleState,
        VehicleStatus,
    },
};

#[derive(Clone, Debug, PartialEq)]
struct RouteSlot {
    generation: u32,
    external_id: String,
    edge_handles: Vec<EdgeHandle>,
    active: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct VehicleSlot {
    generation: u32,
    external_id: String,
    state: Option<VehicleState>,
}

/// LaneFlow Core 的最小 runtime state。
#[derive(Clone, Debug, PartialEq)]
pub struct CoreWorld {
    fixed_delta_time_ms: u64,
    tick_index: u64,
    time_ms: u64,
    lane_graph: LaneGraph,
    vehicle_profiles: VehicleProfileRegistry,
    routes: Vec<RouteSlot>,
    route_handles: IndexMap<String, RouteHandle>,
    free_route_indices: Vec<usize>,
    vehicles: Vec<VehicleSlot>,
    vehicle_handles: IndexMap<String, VehicleHandle>,
    free_vehicle_indices: Vec<usize>,
    vehicle_update_order: Vec<VehicleHandle>,
    occupancy_scratch: OccupancyScratch,
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

        let (lane_graph, routes, vehicle_profiles) = traffic_data.into_parts();
        let mut world = Self {
            fixed_delta_time_ms,
            tick_index: 0,
            time_ms: 0,
            lane_graph,
            vehicle_profiles,
            routes: Vec::new(),
            route_handles: IndexMap::new(),
            free_route_indices: Vec::new(),
            vehicles: Vec::new(),
            vehicle_handles: IndexMap::new(),
            free_vehicle_indices: Vec::new(),
            vehicle_update_order: Vec::new(),
            occupancy_scratch: OccupancyScratch::default(),
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
            .filter_map(|handle| self.vehicle(*handle))
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

        let edge_handles = resolve_route_edges(&self.lane_graph, &route)?;
        let external_id = route.id().to_owned();

        let handle = if let Some(index) = self.free_route_indices.pop() {
            let generation = self.routes[index].generation;
            self.routes[index] = RouteSlot {
                generation,
                external_id: external_id.clone(),
                edge_handles,
                active: true,
            };
            RouteHandle::new(index, generation)
        } else {
            let handle = RouteHandle::new(self.routes.len(), 0);
            self.routes.push(RouteSlot {
                generation: 0,
                external_id: external_id.clone(),
                edge_handles,
                active: true,
            });
            handle
        };

        self.route_handles.insert(external_id, handle);
        Ok(handle)
    }

    /// 移除未被 live vehicle 引用的 route definition。
    pub fn remove_route(&mut self, handle: RouteHandle) -> Result<RouteRemoveRecord, CoreError> {
        self.route_slot(handle)
            .ok_or(CoreError::UnknownRouteHandle { route: handle })?;

        for vehicle in self.vehicles.iter().filter_map(|slot| slot.state.as_ref()) {
            if vehicle.route == handle {
                return Err(CoreError::RouteInUse {
                    route: handle,
                    vehicle: vehicle.handle,
                });
            }
        }

        let route = &mut self.routes[handle.index()];
        let external_id = route.external_id.clone();
        route.active = false;
        route.edge_handles.clear();
        self.route_handles.shift_remove(&external_id);
        // generation 耗尽时不复用 slot，避免 stale handle 在回绕后复活。
        if let Some(next_generation) = route.generation.checked_add(1) {
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
        let external_id = input.id;
        let handle = if let Some(index) = self.free_vehicle_indices.pop() {
            let generation = self.vehicles[index].generation;
            let handle = VehicleHandle::new(index, generation);
            self.vehicles[index] = VehicleSlot {
                generation,
                external_id: external_id.clone(),
                state: Some(VehicleState::new(
                    handle,
                    normalized.profile,
                    route,
                    normalized.route_edge_index,
                    normalized.edge_progress,
                    normalized.current_speed,
                    normalized.status,
                )),
            };
            handle
        } else {
            let handle = VehicleHandle::new(self.vehicles.len(), 0);
            self.vehicles.push(VehicleSlot {
                generation: 0,
                external_id: external_id.clone(),
                state: Some(VehicleState::new(
                    handle,
                    normalized.profile,
                    route,
                    normalized.route_edge_index,
                    normalized.edge_progress,
                    normalized.current_speed,
                    normalized.status,
                )),
            });
            handle
        };

        self.vehicle_handles.insert(external_id, handle);
        self.vehicle_update_order.push(handle);
        Ok(handle)
    }

    /// 移除 live vehicle runtime entity。
    pub fn despawn_vehicle(
        &mut self,
        handle: VehicleHandle,
    ) -> Result<VehicleDespawnRecord, CoreError> {
        self.vehicle_slot(handle)
            .ok_or(CoreError::UnknownVehicleHandle { vehicle: handle })?;

        let slot = &mut self.vehicles[handle.index()];
        let state = slot
            .state
            .take()
            .expect("validated vehicle slot must contain state");
        let external_id = slot.external_id.clone();
        self.vehicle_handles.shift_remove(&external_id);
        // generation 耗尽时不复用 slot，避免 stale handle 在回绕后复活。
        if let Some(next_generation) = slot.generation.checked_add(1) {
            slot.generation = next_generation;
            self.free_vehicle_indices.push(handle.index());
        }
        self.vehicle_update_order
            .retain(|candidate| *candidate != handle);

        Ok(VehicleDespawnRecord {
            handle,
            external_id,
            profile: state.profile,
            route: state.route,
            status: state.status,
        })
    }

    /// 推进一个 fixed-step tick。
    ///
    /// 成功时，`StepResult` 使用 post-step tick/time；失败时 world 保持不变。
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

        self.rebuild_occupancy_and_leaders()?;

        // 为失败原子性只克隆紧凑运行态，避免每 tick 复制 external ID registry 字符串。
        let mut next_vehicle_states: Vec<_> = self
            .vehicles
            .iter()
            .map(|slot| slot.state.clone())
            .collect();
        let mut events = Vec::new();
        for vehicle_handle in &self.vehicle_update_order {
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

            let Some(vehicle) = next_vehicle_states
                .get_mut(vehicle_handle.index())
                .and_then(Option::as_mut)
            else {
                continue;
            };

            Self::advance_vehicle(
                &self.lane_graph,
                &self.routes,
                self.fixed_delta_time_ms,
                next_tick_index,
                vehicle,
                &mut events,
            )?;
        }

        self.tick_index = next_tick_index;
        self.time_ms = next_time_ms;
        for (slot, next_state) in self.vehicles.iter_mut().zip(next_vehicle_states) {
            slot.state = next_state;
        }

        Ok(StepResult {
            tick_index: next_tick_index,
            time_ms: next_time_ms,
            events,
        })
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
        if current_edge == target_edge && target_front_progress >= front_progress {
            let front_distance = target_front_progress - front_progress;
            return (front_distance <= max_front_distance).then_some(front_distance);
        }

        let mut distance = self
            .lane_graph
            .edge_length(current_edge)
            .expect("route edge must exist")
            .value()
            - front_progress;
        if distance > max_front_distance {
            return None;
        }
        for edge in route
            .edge_handles
            .iter()
            .copied()
            .skip(route_edge_index + 1)
        {
            if edge == target_edge {
                let remaining = max_front_distance - distance;
                return (target_front_progress <= remaining)
                    .then_some(distance + target_front_progress);
            }
            let edge_length = self
                .lane_graph
                .edge_length(edge)
                .expect("route edge must exist")
                .value();
            if edge_length > max_front_distance - distance {
                return None;
            }
            distance += edge_length;
        }
        None
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

        for handle in self.vehicle_update_order.iter().copied() {
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

            for handle in self.vehicle_update_order.iter().copied() {
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

    fn build_occupancy(&self, scratch: &mut OccupancyScratch) {
        scratch.begin(self.lane_graph.edges().len(), self.vehicles.len());

        for handle in self.vehicle_update_order.iter().copied() {
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
        for (update_sequence, handle) in self.vehicle_update_order.iter().copied().enumerate() {
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
        let first_ahead = current_occupants
            .partition_point(|occupant| occupant.front_progress <= follower.edge_progress.value());
        for occupant in &current_occupants[first_ahead..] {
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
        lane_graph: &LaneGraph,
        routes: &[RouteSlot],
        fixed_delta_time_ms: u64,
        tick_index: u64,
        vehicle: &mut VehicleState,
        events: &mut Vec<CoreEvent>,
    ) -> Result<(), CoreError> {
        if vehicle.status != VehicleStatus::Active {
            return Ok(());
        }

        vehicle.applied_acceleration = Acceleration::ZERO;
        let speed = vehicle.current_speed.value();
        let delta_time_seconds = fixed_delta_time_ms as f64 / 1_000.0;
        let travel_distance = speed * delta_time_seconds;
        if !travel_distance.is_finite() {
            return Err(CoreError::NonFiniteRouteTravel {
                vehicle: vehicle.handle,
                speed,
                delta_time_ms: fixed_delta_time_ms,
            });
        }
        if travel_distance <= EDGE_BOUNDARY_EPSILON {
            return Ok(());
        }

        let route = routes
            .get(vehicle.route.index())
            .filter(|route| route.active && route.generation == vehicle.route.generation())
            .expect("validated vehicle route must exist");
        let max_iterations = route.edge_handles.len() - vehicle.route_edge_index;
        let mut remaining = travel_distance;

        for _ in 0..max_iterations {
            if is_less_than_boundary_epsilon(remaining) {
                break;
            }

            let current_edge = route
                .edge_handles
                .get(vehicle.route_edge_index)
                .copied()
                .expect("validated route edge index must exist");
            let edge_length = lane_graph
                .edge_length(current_edge)
                .expect("validated route edge must exist")
                .value();
            let next_progress = vehicle.edge_progress.value() + remaining;
            if !next_progress.is_finite() {
                return Err(CoreError::NonFiniteRouteTravel {
                    vehicle: vehicle.handle,
                    speed,
                    delta_time_ms: fixed_delta_time_ms,
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
                let to_edge = route
                    .edge_handles
                    .get(to_route_edge_index)
                    .copied()
                    .expect("next route edge must exist");

                events.push(CoreEvent::VehicleChangedEdge(VehicleChangedEdgeEvent {
                    tick_index,
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
                        tick_index,
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
        world.vehicles[original.index()].generation = u32::MAX;
        world.vehicle_handles.insert("V1".to_owned(), exhausted);

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
