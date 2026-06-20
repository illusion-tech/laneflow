//! Core world 与 fixed-step orchestration。

use crate::{
    error::CoreError,
    graph::LaneGraph,
    route::Route,
    time::{StepResult, TickInput},
    vehicle::VehicleState,
};
use indexmap::IndexMap;

/// LaneFlow Core 的最小 runtime state。
#[derive(Clone, Debug, PartialEq)]
pub struct CoreWorld {
    fixed_delta_time_ms: u64,
    tick_index: u64,
    time_ms: u64,
    lane_graph: LaneGraph,
    routes: IndexMap<String, Route>,
    vehicles: Vec<VehicleState>,
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
        vehicles: Vec<VehicleState>,
    ) -> Result<Self, CoreError>
    where
        I: IntoIterator<Item = Route>,
    {
        if fixed_delta_time_ms == 0 {
            return Err(CoreError::InvalidFixedDeltaTime {
                fixed_delta_time_ms,
            });
        }

        let routes = Self::collect_routes(routes)?;
        Self::validate_routes(&lane_graph, &routes)?;
        Self::validate_vehicles(&lane_graph, &routes, &vehicles)?;

        Ok(Self {
            fixed_delta_time_ms,
            tick_index: 0,
            time_ms: 0,
            lane_graph,
            routes,
            vehicles,
        })
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

    /// 返回当前车辆状态。
    pub fn vehicles(&self) -> &[VehicleState] {
        &self.vehicles
    }

    /// 返回当前 lane graph。
    pub const fn lane_graph(&self) -> &LaneGraph {
        &self.lane_graph
    }

    /// 返回指定 route。
    pub fn route(&self, id: &str) -> Option<&Route> {
        self.routes.get(id)
    }

    /// 返回所有 route，顺序与初始化输入一致。
    pub fn routes(&self) -> impl ExactSizeIterator<Item = &Route> {
        self.routes.values()
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

        self.tick_index = next_tick_index;
        self.time_ms = next_time_ms;

        Ok(StepResult {
            tick_index: self.tick_index,
            time_ms: self.time_ms,
            events: Vec::new(),
        })
    }

    fn collect_routes<I>(routes: I) -> Result<IndexMap<String, Route>, CoreError>
    where
        I: IntoIterator<Item = Route>,
    {
        let mut route_map = IndexMap::new();

        for route in routes {
            let route_id = route.id().to_owned();
            if route_map.contains_key(&route_id) {
                return Err(CoreError::DuplicateRouteId { route_id });
            }
            route_map.insert(route_id, route);
        }

        Ok(route_map)
    }

    fn validate_routes(
        lane_graph: &LaneGraph,
        routes: &IndexMap<String, Route>,
    ) -> Result<(), CoreError> {
        for route in routes.values() {
            for edge_id in route.edge_ids() {
                if lane_graph.edge(edge_id).is_none() {
                    return Err(CoreError::UnknownRouteEdge {
                        route_id: route.id().to_owned(),
                        edge_id: edge_id.clone(),
                    });
                }
            }

            for [from_edge_id, to_edge_id] in route.edge_ids().array_windows::<2>() {
                if !lane_graph.can_traverse(from_edge_id, to_edge_id) {
                    return Err(CoreError::DisconnectedRouteEdge {
                        route_id: route.id().to_owned(),
                        from_edge_id: from_edge_id.clone(),
                        to_edge_id: to_edge_id.clone(),
                    });
                }
            }
        }

        Ok(())
    }

    fn validate_vehicles(
        lane_graph: &LaneGraph,
        routes: &IndexMap<String, Route>,
        vehicles: &[VehicleState],
    ) -> Result<(), CoreError> {
        let mut vehicle_ids = IndexMap::new();

        for vehicle in vehicles {
            if vehicle_ids.insert(vehicle.id.clone(), ()).is_some() {
                return Err(CoreError::DuplicateVehicleId {
                    vehicle_id: vehicle.id.clone(),
                });
            }

            let route =
                routes
                    .get(&vehicle.route_id)
                    .ok_or_else(|| CoreError::UnknownVehicleRoute {
                        vehicle_id: vehicle.id.clone(),
                        route_id: vehicle.route_id.clone(),
                    })?;

            let edge_id = route
                .edge_ids()
                .get(vehicle.route_edge_index)
                .ok_or_else(|| CoreError::InvalidVehicleRouteEdgeIndex {
                    vehicle_id: vehicle.id.clone(),
                    route_id: vehicle.route_id.clone(),
                    route_edge_index: vehicle.route_edge_index,
                    route_edge_count: route.edge_ids().len(),
                })?;

            let edge_length = lane_graph
                .edge_length(edge_id)
                .expect("validated route edge must exist");
            if vehicle.edge_progress.value() > edge_length.value() {
                return Err(CoreError::VehicleEdgeProgressOutOfRange {
                    vehicle_id: vehicle.id.clone(),
                    edge_id: edge_id.clone(),
                    edge_progress: vehicle.edge_progress.value(),
                    edge_length: edge_length.value(),
                });
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CoreError, EdgeLength, EdgeProgress, LaneEdge, Speed, TickInput, VehicleState};

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
        let vehicle = VehicleState::active(
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
}
