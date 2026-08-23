use std::sync::Arc;

use laneflow_static_contract::{
    EntityKind, ParkingSpaceOrdinal, SignalAspect, SignalControllerOrdinal, SignalGroupOrdinal,
    StaticRouteOrdinal,
};
use laneflow_static_network::SharedNetworkRevision;

use crate::tables::{
    CompiledRoute, DynamicRouteSlot, VehicleSlot, VehicleState, VehicleStatus, bodies_overlap,
    compile_dynamic_route, route_access_denied, spawn_motion_error, static_route_ordinal,
};
use crate::{
    CommittedPoseSourceBatch, CommittedSignalGroupBatch, InstallError, LookupError, ParkingError,
    PoseSource, RouteError, RouteHandle, RouteRegisterInput, SpawnError, StepError, StepOutcome,
    TickInput, VehicleHandle, VehicleSpawnInput, WorldConfig,
};

/// 1-worker 交通世界。只克隆根 `Arc`，不复制静态 component。
/// 生命周期命令（`register_route` / `spawn_vehicle` / `occupy_parking`）只在两次 `step` 之间调用。
pub struct TrafficWorld {
    pub(crate) revision: Arc<SharedNetworkRevision>,
    pub(crate) config: WorldConfig,
    pub(crate) tick_index: u64,
    pub(crate) time_ms: u64,
    pub(crate) signal_aspects: Box<[SignalAspect]>,
    pub(crate) dynamic_routes: Vec<DynamicRouteSlot>,
    pub(crate) free_routes: Vec<usize>,
    pub(crate) live_dynamic_routes: u32,
    pub(crate) vehicles: Vec<VehicleSlot>,
    pub(crate) free_vehicles: Vec<usize>,
    pub(crate) live_order: Vec<VehicleHandle>,
    pub(crate) parking_occupants: Box<[Option<VehicleHandle>]>,
    pub(crate) next_states: Vec<(usize, VehicleState)>,
}

impl TrafficWorld {
    /// 安装完整共享根。失败不留下可观察的半个 world。
    pub fn install(
        revision: Arc<SharedNetworkRevision>,
        config: WorldConfig,
    ) -> Result<Self, InstallError> {
        if config.fixed_delta_time_ms() == 0 {
            return Err(InstallError::NonPositiveDelta);
        }
        if config.worker_count() != 1 {
            return Err(InstallError::WorkerCountNotOne);
        }
        validate_signal_programs(revision.as_ref(), config.fixed_delta_time_ms())?;
        let group_count = usize::try_from(
            revision
                .traffic()
                .entity_counts()
                .count(EntityKind::SignalGroup),
        )
        .expect("signal group count fits usize");
        let space_count = usize::try_from(
            revision
                .traffic()
                .entity_counts()
                .count(EntityKind::ParkingSpace),
        )
        .expect("parking space count fits usize");
        let vehicle_capacity = usize::try_from(config.vehicle_capacity()).unwrap_or(0);
        let route_capacity = usize::try_from(config.dynamic_route_capacity()).unwrap_or(0);
        let mut world = Self {
            revision,
            config,
            tick_index: 0,
            time_ms: 0,
            signal_aspects: vec![SignalAspect::Red; group_count].into_boxed_slice(),
            dynamic_routes: Vec::with_capacity(route_capacity),
            free_routes: Vec::with_capacity(route_capacity),
            live_dynamic_routes: 0,
            vehicles: Vec::with_capacity(vehicle_capacity),
            free_vehicles: Vec::with_capacity(vehicle_capacity),
            live_order: Vec::with_capacity(vehicle_capacity),
            parking_occupants: vec![None; space_count].into_boxed_slice(),
            next_states: Vec::with_capacity(vehicle_capacity),
        };
        world.refresh_signals();
        Ok(world)
    }

    /// 共享根。
    #[must_use]
    pub fn revision(&self) -> Arc<SharedNetworkRevision> {
        Arc::clone(&self.revision)
    }

    /// 共享 Traffic component。
    #[must_use]
    pub fn traffic(&self) -> &laneflow_static_network::SharedTrafficNetwork {
        self.revision.traffic()
    }

    /// 已提交 `tick_index`。`install` 后为 0；成功 `step` 与 `StepOutcome` 一致；失败不变。
    #[must_use]
    pub const fn tick_index(&self) -> u64 {
        self.tick_index
    }

    /// 已提交 `time_ms`。`install` 后为 0；成功 `step` 与 `StepOutcome` 一致；失败不变。
    #[must_use]
    pub const fn time_ms(&self) -> u64 {
        self.time_ms
    }

    /// 安装时冻结的 world 配置。
    #[must_use]
    pub const fn config(&self) -> WorldConfig {
        self.config
    }

    /// 取得 compiler 预编译静态路线句柄。
    pub fn static_route(&self, route: StaticRouteOrdinal) -> Result<RouteHandle, LookupError> {
        let count = self
            .revision
            .traffic()
            .entity_counts()
            .count(EntityKind::StaticRoute);
        if route.raw() >= count {
            return Err(LookupError::UnknownStaticRoute);
        }
        Ok(RouteHandle::static_route(route.raw()))
    }

    /// 注册本世界动态路线。失败不留下半条路线。
    pub fn register_route(&mut self, input: RouteRegisterInput) -> Result<RouteHandle, RouteError> {
        if self.live_dynamic_routes >= self.config.dynamic_route_capacity() {
            return Err(RouteError::CapacityExceeded);
        }
        let compiled = compile_dynamic_route(self.revision.traffic(), input.edges())?;
        let slot_index = self.free_routes.pop().unwrap_or(self.dynamic_routes.len());
        let generation = self
            .dynamic_routes
            .get(slot_index)
            .map_or(0, |slot| slot.generation);
        let handle = RouteHandle::dynamic_route(
            u32::try_from(slot_index).expect("dynamic route index fits u32"),
            generation,
        );
        let slot = DynamicRouteSlot {
            generation,
            compiled: Some(compiled),
            live_vehicles: 0,
        };
        if slot_index == self.dynamic_routes.len() {
            self.dynamic_routes.push(slot);
        } else {
            self.dynamic_routes[slot_index] = slot;
        }
        self.live_dynamic_routes += 1;
        Ok(handle)
    }

    /// 只移除本世界动态路线。静态句柄必须拒绝。
    pub fn remove_route(&mut self, route: RouteHandle) -> Result<(), RouteError> {
        if route.is_static() {
            return Err(RouteError::StaticHandle);
        }
        let index = usize::try_from(route.index()).expect("dynamic route index fits usize");
        let Some(slot) = self.dynamic_routes.get_mut(index) else {
            return Err(RouteError::StaleHandle);
        };
        if slot.generation != route.generation() || slot.compiled.is_none() {
            return Err(RouteError::StaleHandle);
        }
        if slot.live_vehicles > 0 {
            let vehicle = self
                .live_order
                .iter()
                .copied()
                .find(|vehicle| {
                    self.vehicle_state(*vehicle)
                        .is_some_and(|state| state.route == route)
                })
                .expect("live_vehicles > 0 必须能找到引用车辆");
            return Err(RouteError::InUse { vehicle, route });
        }
        slot.compiled = None;
        if let Some(next_generation) = slot.generation.checked_add(1) {
            slot.generation = next_generation;
            self.free_routes.push(index);
            self.live_dynamic_routes = self.live_dynamic_routes.saturating_sub(1);
        }
        Ok(())
    }

    /// 生成一辆车。失败不留半辆车。
    pub fn spawn_vehicle(&mut self, input: VehicleSpawnInput) -> Result<VehicleHandle, SpawnError> {
        let live = u32::try_from(self.live_order.len()).expect("live vehicle count fits u32");
        if live >= self.config.vehicle_capacity() {
            return Err(SpawnError::CapacityExceeded);
        }
        if let Some(error) = spawn_motion_error(input.progress(), input.initial_speed()) {
            return Err(error);
        }
        let profile = self
            .revision
            .traffic()
            .relations()
            .vehicle_profile(input.profile())
            .ok_or(SpawnError::UnknownProfile)?;
        let cursor = usize::try_from(input.route_edge_index()).expect("route index fits usize");
        let edge = {
            let edges = self
                .route_edges(input.route())
                .ok_or(SpawnError::UnknownRoute)?;
            *edges.get(cursor).ok_or(SpawnError::RouteIndexOutOfRange)?
        };
        let length = self.revision.traffic().lane_lengths_meters()[edge.index()];
        if input.progress() > length {
            return Err(SpawnError::InvalidProgress);
        }
        let speed_limit = self
            .revision
            .traffic()
            .lane_speed_limits_meters_per_second()[edge.index()];
        if input.initial_speed() > speed_limit {
            return Err(SpawnError::SpeedExceedsLimit);
        }
        if self.route_suffix_denied(input.route(), profile.class(), cursor) {
            return Err(SpawnError::AccessDenied);
        }
        if self.lane_overlap(input.route(), cursor, input.progress(), profile.length()) {
            return Err(SpawnError::Overlap);
        }

        let slot_index = self.free_vehicles.pop().unwrap_or(self.vehicles.len());
        let generation = self
            .vehicles
            .get(slot_index)
            .map_or(0, |slot| slot.generation);
        let handle = VehicleHandle::new(
            u32::try_from(slot_index).expect("vehicle index fits u32"),
            generation,
        );
        let state = VehicleState {
            handle,
            profile: input.profile(),
            class: profile.class(),
            route: input.route(),
            route_edge_index: input.route_edge_index(),
            progress: input.progress(),
            speed: input.initial_speed(),
            length: profile.length(),
            status: VehicleStatus::Active,
            parking: None,
        };
        let slot = VehicleSlot {
            generation,
            state: Some(state),
        };
        if slot_index == self.vehicles.len() {
            self.vehicles.push(slot);
        } else {
            self.vehicles[slot_index] = slot;
        }
        if !input.route().is_static() {
            let route_index =
                usize::try_from(input.route().index()).expect("route index fits usize");
            self.dynamic_routes[route_index].live_vehicles += 1;
        }
        self.live_order.push(handle);
        Ok(handle)
    }

    /// 停车占用：每车一个车位、每车位一车；同车位幂等。
    pub fn occupy_parking(
        &mut self,
        vehicle: VehicleHandle,
        space: ParkingSpaceOrdinal,
    ) -> Result<(), ParkingError> {
        let space_index = space.index();
        let Some(occupant_slot) = self.parking_occupants.get(space_index) else {
            return Err(ParkingError::UnknownSpace);
        };
        let Some(state) = self.vehicle_state(vehicle) else {
            return Err(ParkingError::UnknownVehicle);
        };
        if let Some(current) = state.parking {
            if current == space {
                return Ok(());
            }
            return Err(ParkingError::VehicleBoundToOtherSpace);
        }
        if let Some(other) = *occupant_slot {
            if other == vehicle {
                return Ok(());
            }
            return Err(ParkingError::SpaceOccupiedByOther);
        }
        if state.status != VehicleStatus::Active {
            return Err(ParkingError::UnknownVehicle);
        }
        let route = state.route;
        let slot_index = usize::try_from(vehicle.index()).expect("vehicle index fits usize");
        let state = self.vehicles[slot_index]
            .state
            .as_mut()
            .expect("resolved vehicle remains live");
        state.status = VehicleStatus::Parked;
        state.speed = 0.0;
        state.parking = Some(space);
        self.parking_occupants[space_index] = Some(vehicle);
        self.release_route_ref(route);
        Ok(())
    }

    /// 固定步进。`delta_time_ms` 必须等于 `WorldConfig.fixed_delta_time_ms`；
    /// `tick_index`/`time_ms` 用 checked 加法。运动、跟车与信号遵守只读 snapshot(T)；
    /// 成功后再提交 T+D 的 pose、时间与 `committed_signal_groups`。相位边界落在
    /// `[T, T+D)` 时该拍仍用 snapshot(T) 灯色。失败不推进时间，已提交查询与失败前一致。
    /// 生命周期命令只在两次 `step` 之间调用。
    pub fn step(&mut self, input: TickInput) -> Result<StepOutcome, StepError> {
        self.step_vehicles(input)
    }

    /// 稳定顺序的已提交 pose 源。
    #[must_use]
    pub fn committed_pose_sources(&self) -> CommittedPoseSourceBatch {
        let items = self
            .live_order
            .iter()
            .copied()
            .filter_map(|handle| {
                let state = self.vehicle_state(handle)?;
                let source = match state.status {
                    VehicleStatus::Completed => return None,
                    VehicleStatus::Parked => PoseSource::Parking {
                        space: state.parking.expect("parked vehicle has a space"),
                    },
                    VehicleStatus::Active => {
                        let edges = self.route_edges(state.route)?;
                        let edge = *edges.get(usize::try_from(state.route_edge_index).ok()?)?;
                        PoseSource::Lane {
                            edge,
                            progress: state.progress,
                        }
                    }
                };
                Some((handle, source))
            })
            .collect();
        CommittedPoseSourceBatch { items }
    }

    /// 按停车位序号读占用者。
    #[must_use]
    pub fn committed_parking_occupant(&self, space: ParkingSpaceOrdinal) -> Option<VehicleHandle> {
        self.parking_occupants.get(space.index()).copied().flatten()
    }

    /// 稳定按组序号的当前 aspect。
    #[must_use]
    pub fn committed_signal_groups(&self) -> CommittedSignalGroupBatch {
        let items = self
            .signal_aspects
            .iter()
            .enumerate()
            .map(|(index, aspect)| {
                (
                    SignalGroupOrdinal::from_raw(
                        u32::try_from(index).expect("signal group index fits u32"),
                    ),
                    *aspect,
                )
            })
            .collect();
        CommittedSignalGroupBatch { items }
    }

    pub(crate) fn vehicle_state(&self, handle: VehicleHandle) -> Option<&VehicleState> {
        let slot = self.vehicles.get(usize::try_from(handle.index()).ok()?)?;
        if slot.generation != handle.generation() {
            return None;
        }
        slot.state.as_ref()
    }

    pub(crate) fn route_edges(
        &self,
        route: RouteHandle,
    ) -> Option<&[laneflow_static_contract::LaneEdgeOrdinal]> {
        if let Some(ordinal) = static_route_ordinal(route) {
            return self
                .revision
                .traffic()
                .relations()
                .static_route_edges(ordinal);
        }
        let slot = self
            .dynamic_routes
            .get(usize::try_from(route.index()).ok()?)?;
        if slot.generation != route.generation() {
            return None;
        }
        Some(slot.compiled.as_ref()?.edges.as_ref())
    }

    pub(crate) fn compiled_dynamic_route(&self, route: RouteHandle) -> Option<&CompiledRoute> {
        if static_route_ordinal(route).is_some() {
            return None;
        }
        let slot = self
            .dynamic_routes
            .get(usize::try_from(route.index()).ok()?)?;
        if slot.generation != route.generation() {
            return None;
        }
        slot.compiled.as_ref()
    }

    fn route_suffix_denied(
        &self,
        route: RouteHandle,
        class: laneflow_static_contract::ParticipantClassOrdinal,
        cursor: usize,
    ) -> bool {
        let Some(edges) = self.route_edges(route) else {
            return true;
        };
        if let Some(ordinal) = static_route_ordinal(route) {
            let relations = self.revision.traffic().relations();
            let count = relations.route_maneuver_count(ordinal).unwrap_or(0);
            let maneuvers = (0..count).filter_map(move |index| {
                let occurrence = relations.route_maneuver_occurrence(ordinal, index)?;
                Some((occurrence.path(), occurrence.exit_route_edge_index()))
            });
            return route_access_denied(self.revision.traffic(), class, edges, cursor, maneuvers);
        }
        let Some(slot) = self
            .dynamic_routes
            .get(usize::try_from(route.index()).expect("dynamic route index fits usize"))
        else {
            return true;
        };
        if slot.generation != route.generation() {
            return true;
        }
        let Some(compiled) = slot.compiled.as_ref() else {
            return true;
        };
        route_access_denied(
            self.revision.traffic(),
            class,
            edges,
            cursor,
            compiled
                .maneuvers
                .iter()
                .map(|occurrence| (occurrence.path, occurrence.exit_route_edge_index)),
        )
    }

    fn lane_overlap(&self, route: RouteHandle, cursor: usize, progress: f64, length: f64) -> bool {
        let Some(spawn_edges) = self.route_edges(route) else {
            return true;
        };
        let lengths = self.revision.traffic().lane_lengths_meters();
        self.live_order.iter().copied().any(|handle| {
            let Some(state) = self.vehicle_state(handle) else {
                return false;
            };
            if state.status != VehicleStatus::Active {
                return false;
            }
            let Some(edges) = self.route_edges(state.route) else {
                return false;
            };
            let Ok(index) = usize::try_from(state.route_edge_index) else {
                return false;
            };
            bodies_overlap(
                lengths,
                spawn_edges,
                cursor,
                progress,
                length,
                edges,
                index,
                state.progress,
                state.length,
            )
        })
    }

    pub(crate) fn release_route_ref(&mut self, route: RouteHandle) {
        if route.is_static() {
            return;
        }
        let Ok(index) = usize::try_from(route.index()) else {
            return;
        };
        let Some(slot) = self.dynamic_routes.get_mut(index) else {
            return;
        };
        if slot.generation != route.generation() || slot.compiled.is_none() {
            return;
        }
        slot.live_vehicles = slot.live_vehicles.saturating_sub(1);
    }

    pub(crate) fn retire_completed_vehicle(&mut self, slot: usize, handle: VehicleHandle) {
        self.live_order.retain(|current| *current != handle);
        let Some(vehicle) = self.vehicles.get_mut(slot) else {
            return;
        };
        vehicle.state = None;
        if let Some(next_generation) = vehicle.generation.checked_add(1) {
            vehicle.generation = next_generation;
            self.free_vehicles.push(slot);
        }
    }

    pub(crate) fn refresh_signals(&mut self) {
        fill_signal_aspects(
            self.revision.as_ref(),
            self.time_ms,
            &mut self.signal_aspects,
        );
    }
}

pub(crate) fn fill_signal_aspects(
    revision: &SharedNetworkRevision,
    time_ms: u64,
    aspects: &mut [SignalAspect],
) {
    aspects.fill(SignalAspect::Red);
    let relations = revision.traffic().relations();
    let controller_count = revision
        .traffic()
        .entity_counts()
        .count(EntityKind::SignalController);
    for raw in 0..controller_count {
        let controller = SignalControllerOrdinal::from_raw(raw);
        let Some(view) = relations.signal_controller(controller) else {
            continue;
        };
        let cycle_ms = view.cycle_ms();
        if cycle_ms == 0 || view.phases().is_empty() {
            continue;
        }
        let position = u64::try_from(
            (u128::from(time_ms) + u128::from(view.offset_ms())) % u128::from(cycle_ms),
        )
        .expect("cycle position fits u64");
        let phases = view.phases();
        let phase_index = phases.partition_point(|phase| {
            relations.phase_end_offset_ms(*phase).unwrap_or(0) <= position
        });
        let Some(phase) = phases.get(phase_index).copied() else {
            continue;
        };
        let Some((groups, values)) = relations.phase_states(phase) else {
            continue;
        };
        for (group, aspect) in groups.iter().copied().zip(values.iter().copied()) {
            if let Some(slot) = aspects.get_mut(group.index()) {
                *slot = aspect;
            }
        }
    }
}

fn validate_signal_programs(
    revision: &SharedNetworkRevision,
    fixed_delta_time_ms: u64,
) -> Result<(), InstallError> {
    let relations = revision.traffic().relations();
    let controller_count = revision
        .traffic()
        .entity_counts()
        .count(EntityKind::SignalController);
    for raw in 0..controller_count {
        let controller = SignalControllerOrdinal::from_raw(raw);
        let Some(view) = relations.signal_controller(controller) else {
            return Err(InstallError::InvalidSignalProgram);
        };
        if view.cycle_ms() == 0 || view.phases().is_empty() {
            return Err(InstallError::InvalidSignalProgram);
        }
        for phase in view.phases() {
            let Some(duration_ms) = relations.phase_duration_ms(*phase) else {
                return Err(InstallError::InvalidSignalProgram);
            };
            if duration_ms < fixed_delta_time_ms {
                return Err(InstallError::PhaseShorterThanTick);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod overflow_tests {
    use super::*;
    use laneflow_format::{FormatLimits, check_canonical_network_input_v1};
    use laneflow_static_network::{
        SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
        build_shared_network_revision,
    };

    const FULL_SPATIAL: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable-v1/lfca-v1-full-spatial/expected.lfca"
    );

    fn world() -> TrafficWorld {
        let input = check_canonical_network_input_v1(FULL_SPATIAL, FormatLimits::V1_HARD)
            .expect("checked canonical network input");
        let revision = build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::RetainAvailable,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .expect("shared network revision");
        TrafficWorld::install(revision, WorldConfig::new(8, 4, 1, 100)).expect("install")
    }

    #[test]
    fn step_rejects_tick_and_time_overflow() {
        let mut world = world();
        world.tick_index = u64::MAX;
        world.time_ms = 0;
        assert_eq!(
            world.step(TickInput::new(100)).unwrap_err(),
            StepError::Overflow
        );
        assert_eq!(world.tick_index, u64::MAX);
        assert_eq!(world.time_ms, 0);

        world.tick_index = 0;
        world.time_ms = u64::MAX;
        assert_eq!(
            world.step(TickInput::new(100)).unwrap_err(),
            StepError::Overflow
        );
        assert_eq!(world.tick_index, 0);
        assert_eq!(world.time_ms, u64::MAX);
    }
}
