//! Core world 与 fixed-step orchestration。

use indexmap::IndexMap;

use crate::{
    error::CoreError,
    event::{CoreEvent, VehicleChangedEdgeEvent, VehicleCompletedRouteEvent},
    graph::{EDGE_BOUNDARY_EPSILON, LaneGraph},
    handle::{EdgeHandle, RouteHandle, VehicleHandle},
    id::validate_external_id,
    route::{Route, RouteRemoveRecord},
    time::{StepResult, TickInput},
    vehicle::{
        EdgeProgress, Speed, VehicleDespawnRecord, VehicleSpawnInput, VehicleState, VehicleStatus,
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
    routes: Vec<RouteSlot>,
    route_handles: IndexMap<String, RouteHandle>,
    free_route_indices: Vec<usize>,
    vehicles: Vec<VehicleSlot>,
    vehicle_handles: IndexMap<String, VehicleHandle>,
    free_vehicle_indices: Vec<usize>,
    vehicle_update_order: Vec<VehicleHandle>,
}

impl CoreWorld {
    /// 创建不含 traffic data 和车辆的 Core world。
    pub fn new(fixed_delta_time_ms: u64) -> Result<Self, CoreError> {
        Self::with_traffic_data(
            fixed_delta_time_ms,
            LaneGraph::empty(),
            Vec::new(),
            Vec::new(),
        )
    }

    /// 创建包含 lane graph、routes 和初始车辆的 Core world。
    pub fn with_traffic_data<I>(
        fixed_delta_time_ms: u64,
        lane_graph: LaneGraph,
        routes: I,
        mut vehicles: Vec<VehicleSpawnInput>,
    ) -> Result<Self, CoreError>
    where
        I: IntoIterator<Item = Route>,
    {
        if fixed_delta_time_ms == 0 {
            return Err(CoreError::InvalidFixedDeltaTime {
                fixed_delta_time_ms,
            });
        }

        let mut world = Self {
            fixed_delta_time_ms,
            tick_index: 0,
            time_ms: 0,
            lane_graph,
            routes: Vec::new(),
            route_handles: IndexMap::new(),
            free_route_indices: Vec::new(),
            vehicles: Vec::new(),
            vehicle_handles: IndexMap::new(),
            free_vehicle_indices: Vec::new(),
            vehicle_update_order: Vec::new(),
        };

        for route in routes {
            world.register_route(route)?;
        }

        vehicles.sort_by(|left, right| left.id.cmp(&right.id));
        for vehicle in vehicles {
            world.spawn_vehicle(vehicle)?;
        }

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

        let edge_handles = self.resolve_route_edges(&route)?;
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
        route.generation = route.generation.wrapping_add(1);
        self.route_handles.shift_remove(&external_id);
        self.free_route_indices.push(handle.index());

        Ok(RouteRemoveRecord {
            handle,
            external_id,
        })
    }

    /// 创建新的 vehicle runtime entity。
    pub fn spawn_vehicle(&mut self, input: VehicleSpawnInput) -> Result<VehicleHandle, CoreError> {
        validate_external_id("vehicles[].id", &input.id)?;
        validate_external_id("vehicles[].routeId", &input.route_id)?;
        if self.vehicle_handles.contains_key(&input.id) {
            return Err(CoreError::DuplicateVehicleId {
                vehicle_id: input.id,
            });
        }

        let route =
            self.route_handle(&input.route_id)
                .ok_or_else(|| CoreError::UnknownVehicleRoute {
                    vehicle_id: input.id.clone(),
                    route_id: input.route_id.clone(),
                })?;
        let normalized = self.normalize_vehicle_input(route, &input)?;
        let external_id = input.id;
        let handle = if let Some(index) = self.free_vehicle_indices.pop() {
            let generation = self.vehicles[index].generation;
            let handle = VehicleHandle::new(index, generation);
            self.vehicles[index] = VehicleSlot {
                generation,
                external_id: external_id.clone(),
                state: Some(VehicleState::new(
                    handle,
                    route,
                    normalized.route_edge_index,
                    normalized.edge_progress,
                    normalized.speed,
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
                    route,
                    normalized.route_edge_index,
                    normalized.edge_progress,
                    normalized.speed,
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
        slot.generation = slot.generation.wrapping_add(1);
        self.vehicle_handles.shift_remove(&external_id);
        self.free_vehicle_indices.push(handle.index());
        self.vehicle_update_order
            .retain(|candidate| *candidate != handle);

        Ok(VehicleDespawnRecord {
            handle,
            external_id,
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

    fn resolve_route_edges(&self, route: &Route) -> Result<Vec<EdgeHandle>, CoreError> {
        let mut edge_handles = Vec::with_capacity(route.edge_ids().len());
        for edge_id in route.edge_ids() {
            let edge = self.lane_graph.edge_handle(edge_id).ok_or_else(|| {
                CoreError::UnknownRouteEdge {
                    route_id: route.id().to_owned(),
                    edge_id: edge_id.clone(),
                }
            })?;
            edge_handles.push(edge);
        }

        for [from_edge, to_edge] in edge_handles.array_windows::<2>() {
            if !self.lane_graph.can_traverse(*from_edge, *to_edge) {
                return Err(CoreError::DisconnectedRouteEdge {
                    route_id: route.id().to_owned(),
                    from_edge_id: self
                        .lane_graph
                        .edge_external_id(*from_edge)
                        .expect("resolved route edge must exist")
                        .to_owned(),
                    to_edge_id: self
                        .lane_graph
                        .edge_external_id(*to_edge)
                        .expect("resolved route edge must exist")
                        .to_owned(),
                });
            }
        }

        Ok(edge_handles)
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
            route_edge_index: input.route_edge_index,
            edge_progress,
            speed: input.speed,
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

        let speed = vehicle.effective_speed().value();
        let travel_distance = speed * fixed_delta_time_ms as f64 / 1000.0;
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
    route_edge_index: usize,
    edge_progress: EdgeProgress,
    speed: Speed,
    status: VehicleStatus,
}

fn is_less_than_boundary_epsilon(value: f64) -> bool {
    value < EDGE_BOUNDARY_EPSILON
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CoreError, EdgeLength, EdgeProgress, LaneEdge, Speed, TickInput};

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
        let vehicle = VehicleSpawnInput::active(
            "V1",
            "R1",
            0,
            EdgeProgress::try_new(1.0).expect("valid progress"),
            Speed::try_new(0.0).expect("valid speed"),
        );
        let mut world = CoreWorld::with_traffic_data(20, lane_graph, [route], vec![vehicle])
            .expect("valid world");
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
}
