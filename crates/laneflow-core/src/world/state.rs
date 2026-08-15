use super::*;

/// LaneFlow Core 的最小 runtime state。
#[derive(Clone, Debug, PartialEq)]
pub struct CoreWorld {
    pub(super) fixed_delta_time_ms: u64,
    pub(super) tick_index: u64,
    pub(super) time_ms: u64,
    pub(super) lane_graph: LaneGraph,
    pub(super) vehicle_profiles: VehicleProfileRegistry,
    pub(super) junctions: JunctionRegistry,
    pub(super) signals: SignalRegistry,
    pub(super) parking: ParkingRegistry,
    pub(super) participant_classes: ParticipantClassRegistry,
    pub(super) cross_section: CrossSectionRegistry,
    pub(super) access: AccessRegistry,
    pub(super) waiting: WaitingRegistry,
    pub(crate) parking_runtime: ParkingRuntimeState,
    pub(super) signal_state: SignalRuntimeState,
    pub(super) signal_candidate_scratch: SignalRuntimeScratch,
    pub(super) routes: Vec<RouteSlot>,
    pub(super) route_distance_indices: Vec<RouteDistanceIndex>,
    pub(super) route_reference_indices: Vec<RouteReferenceIndex>,
    pub(super) route_handles: IndexMap<String, RouteHandle>,
    pub(super) free_route_indices: Vec<usize>,
    pub(super) vehicles: Vec<VehicleSlot>,
    pub(super) vehicle_handles: IndexMap<String, VehicleHandle>,
    pub(super) free_vehicle_indices: Vec<usize>,
    pub(super) vehicle_update_order: StableVehicleOrder,
    pub(super) candidate_state_scratch: CandidateStateScratch,
    pub(super) occupancy_scratch: OccupancyScratch,
    pub(super) longitudinal_scratch: LongitudinalScratch,
    pub(super) command_spatial_index: CommandSpatialIndex,
    #[cfg(any(test, feature = "test-support"))]
    pub(super) step_failure_after_vehicle: Option<VehicleHandle>,
    #[cfg(any(test, feature = "test-support"))]
    pub(super) replace_failure_after_prepare: bool,
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

        let (
            lane_graph,
            routes,
            vehicle_profiles,
            junctions,
            signals,
            parking,
            participant_classes,
            cross_section,
            access,
            waiting,
        ) = traffic_data.into_parts();
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
            junctions,
            signals,
            parking,
            participant_classes,
            cross_section,
            access,
            waiting,
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
            #[cfg(any(test, feature = "test-support"))]
            step_failure_after_vehicle: None,
            #[cfg(any(test, feature = "test-support"))]
            replace_failure_after_prepare: false,
        };

        for route in routes {
            world.register_compiled_route(route)?;
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

    /// 返回 immutable Junction registry。
    pub const fn junctions(&self) -> &JunctionRegistry {
        &self.junctions
    }

    /// 返回 immutable Signals registry。
    pub const fn signals(&self) -> &SignalRegistry {
        &self.signals
    }

    /// 返回 immutable WaitingZone registry。
    pub const fn waiting(&self) -> &WaitingRegistry {
        &self.waiting
    }

    /// 返回 immutable Parking registry。
    pub const fn parking(&self) -> &ParkingRegistry {
        &self.parking
    }

    /// 返回 immutable ParticipantClass registry。
    pub const fn participant_classes(&self) -> &ParticipantClassRegistry {
        &self.participant_classes
    }

    /// 返回 immutable CrossSection registry。
    pub const fn cross_section(&self) -> &CrossSectionRegistry {
        &self.cross_section
    }

    /// 返回 immutable Access registry。
    pub const fn access(&self) -> &AccessRegistry {
        &self.access
    }

    /// 返回借用当前 committed world 的 immutable Parking snapshot。
    pub const fn parking_snapshot(&self) -> ParkingSnapshot<'_> {
        ParkingSnapshot::new(self)
    }

    pub(super) fn route_slot(&self, handle: RouteHandle) -> Option<&RouteSlot> {
        self.routes
            .get(handle.index())
            .filter(|route| route.active && route.generation == handle.generation())
    }

    pub(super) fn vehicle_slot(&self, handle: VehicleHandle) -> Option<&VehicleSlot> {
        self.vehicles
            .get(handle.index())
            .filter(|vehicle| vehicle.generation == handle.generation() && vehicle.state.is_some())
    }
}
