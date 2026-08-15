use super::*;

impl CoreWorld {
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

    /// 返回 Route registration-time 编译的 ManeuverPath occurrences。
    pub fn route_maneuver_occurrences(&self, handle: RouteHandle) -> Option<&[ManeuverOccurrence]> {
        self.route_slot(handle)
            .map(|route| route.maneuver_occurrences.as_slice())
    }

    /// 返回 Route registration-time 编译的 Gate occurrences。
    pub fn route_gate_occurrences(&self, handle: RouteHandle) -> Option<&[GateOccurrence]> {
        self.route_slot(handle)
            .map(|route| route.gate_occurrences.as_slice())
    }

    /// 返回 Route registration-time 编译的 WaitingZone occurrences。
    pub fn route_waiting_zone_occurrences(
        &self,
        handle: RouteHandle,
    ) -> Option<&[WaitingZoneOccurrence]> {
        self.route_slot(handle)
            .map(|route| route.waiting_zone_occurrences.as_slice())
    }

    /// 返回指定 Route transition 编译得到的 optional ManeuverGate。
    pub fn route_transition_gate(
        &self,
        handle: RouteHandle,
        from_route_edge_index: usize,
    ) -> Option<Option<ManeuverGateHandle>> {
        self.route_slot(handle)
            .and_then(|route| route.transitions.get(from_route_edge_index))
            .map(|transition| transition.gate)
    }

    /// 返回所有 active route handle，顺序与注册顺序一致。
    pub fn routes(&self) -> impl Iterator<Item = RouteHandle> + '_ {
        self.routes
            .iter()
            .enumerate()
            .filter(|(_, route)| route.active)
            .map(|(index, route)| RouteHandle::new(index, route.generation))
    }

}
