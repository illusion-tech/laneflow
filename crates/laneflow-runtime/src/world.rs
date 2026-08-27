use std::sync::Arc;

use laneflow_static_contract::{
    EntityKind, ParkingSpaceOrdinal, SignalAspect, SignalControllerOrdinal, SignalGroupOrdinal,
};
use laneflow_static_network::SharedNetworkRevision;

use crate::occupancy::OccupancyIndex;
use crate::tables::{
    CompiledRoute, RouteSlot, VehicleSlot, bodies_overlap, compile_route, occupancy_front_gap,
    route_access_denied,
};
use crate::{
    CommittedNetworkSource, CommittedPoseSourceBatch, CommittedSignalGroupBatch, InstallError,
    ParkingError, PoseSource, ReplaceError, RouteError, RouteHandle, RouteRegisterInput,
    SpawnError, StepError, StepOutcome, TickInput, VehicleHandle, VehicleReplaceBlock,
    VehicleReplaceRecord, VehicleSpawnInput, VehicleState, VehicleStatus, WorldConfig,
};

/// 1-worker 交通世界。只克隆根 `Arc`，不复制静态 component。
/// 生命周期命令（`register_route` / `spawn_vehicle` / `occupy_parking` /
/// `replace_completed_vehicle`）只在两次 `step` 之间调用。
pub struct TrafficWorld {
    pub(crate) revision: Arc<SharedNetworkRevision>,
    pub(crate) source: CommittedNetworkSource,
    pub(crate) config: WorldConfig,
    pub(crate) tick_index: u64,
    pub(crate) time_ms: u64,
    pub(crate) signal_aspects: Box<[SignalAspect]>,
    pub(crate) routes: Vec<RouteSlot>,
    pub(crate) free_routes: Vec<usize>,
    pub(crate) live_route_count: u32,
    pub(crate) vehicles: Vec<VehicleSlot>,
    pub(crate) free_vehicles: Vec<usize>,
    pub(crate) live_order: Vec<VehicleHandle>,
    pub(crate) parking_occupants: Box<[Option<VehicleHandle>]>,
    pub(crate) next_states: Vec<(usize, VehicleState)>,
    pub(crate) occupancy: OccupancyIndex,
}

impl TrafficWorld {
    /// 安装完整共享根并指名已提交来源（#302 活动聚合）。失败不留下
    /// 可观察的半个 world。
    ///
    /// 来源的 `NetworkRevisionId` 必须与共享根 origin 精确相等；digest /
    /// length 差异（同修订重发布）按合同只承担来源审计，不构成拒绝条件。
    pub fn install(
        revision: Arc<SharedNetworkRevision>,
        config: WorldConfig,
        source: CommittedNetworkSource,
    ) -> Result<Self, InstallError> {
        let installed = revision.canonical_origin().network_revision();
        if source.network_revision() != installed {
            return Err(InstallError::SourceRevisionMismatch {
                source_revision: source.network_revision(),
                installed_revision: installed,
            });
        }
        let dt = config.fixed_delta_time_ms();
        if !(4..=1_000).contains(&dt) {
            return Err(InstallError::DeltaOutOfRange {
                actual: dt,
                min: 4,
                max: 1_000,
            });
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
        let route_capacity = usize::try_from(config.route_capacity()).unwrap_or(0);
        let mut world = Self {
            revision,
            source,
            config,
            tick_index: 0,
            time_ms: 0,
            signal_aspects: vec![SignalAspect::Red; group_count].into_boxed_slice(),
            routes: Vec::with_capacity(route_capacity),
            free_routes: Vec::with_capacity(route_capacity),
            live_route_count: 0,
            vehicles: Vec::with_capacity(vehicle_capacity),
            free_vehicles: Vec::with_capacity(vehicle_capacity),
            live_order: Vec::with_capacity(vehicle_capacity),
            parking_occupants: vec![None; space_count].into_boxed_slice(),
            next_states: Vec::with_capacity(vehicle_capacity),
            occupancy: OccupancyIndex::with_capacity(0, 0),
        };
        world.refresh_signals();
        Ok(world)
    }

    /// 已提交路网来源（#302 活动聚合的来源指名）。
    #[must_use]
    pub const fn committed_source(&self) -> &CommittedNetworkSource {
        &self.source
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

    /// 注册本世界路线。失败不留下半条路线。
    ///
    /// 在 compiled 槽位物化分段 `u32` 前缀、后缀距离、受控 hop 链和限速下降转换；
    /// 不上 `u64`，不存当前红灯。句柄不含 world 身份，只在本 `TrafficWorld` 内有效。
    pub fn register_route(&mut self, input: RouteRegisterInput) -> Result<RouteHandle, RouteError> {
        if self.live_route_count >= self.config.route_capacity() {
            return Err(RouteError::CapacityExceeded);
        }
        let compiled = compile_route(self.revision.traffic(), input.edges())?;
        let slot_index = self.free_routes.pop().unwrap_or(self.routes.len());
        let generation = self
            .routes
            .get(slot_index)
            .map_or(0, |slot| slot.generation);
        let handle = RouteHandle::new(
            u32::try_from(slot_index).expect("route index fits u32"),
            generation,
        );
        let slot = RouteSlot {
            generation,
            compiled: Some(compiled),
            live_vehicles: 0,
        };
        if slot_index == self.routes.len() {
            self.routes.push(slot);
        } else {
            self.routes[slot_index] = slot;
        }
        self.live_route_count += 1;
        Ok(handle)
    }

    /// 只移除本世界已注册路线。
    pub fn remove_route(&mut self, route: RouteHandle) -> Result<(), RouteError> {
        let index = usize::try_from(route.index()).expect("route index fits usize");
        let Some(slot) = self.routes.get_mut(index) else {
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
        self.live_route_count = self.live_route_count.saturating_sub(1);
        if let Some(next_generation) = slot.generation.checked_add(1) {
            slot.generation = next_generation;
            self.free_routes.push(index);
        }
        Ok(())
    }

    /// 生成一辆车。失败不留半辆车。
    pub fn spawn_vehicle(&mut self, input: VehicleSpawnInput) -> Result<VehicleHandle, SpawnError> {
        let live = u32::try_from(self.live_order.len()).expect("live vehicle count fits u32");
        if live >= self.config.vehicle_capacity() {
            return Err(SpawnError::CapacityExceeded);
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
        let length_mm = self.revision.traffic().lane_lengths_millimetres()[edge.index()];
        if input.progress_mm() > length_mm {
            return Err(SpawnError::InvalidProgress);
        }
        let speed_limit = self
            .revision
            .traffic()
            .lane_speed_limits_millimetres_per_second()[edge.index()];
        if input.initial_speed_mm_s() > speed_limit {
            return Err(SpawnError::SpeedExceedsLimit);
        }
        if self.route_suffix_denied(input.route(), profile.class(), cursor) {
            return Err(SpawnError::AccessDenied);
        }
        if self
            .overlap_blocker(
                input.route(),
                cursor,
                input.progress_mm(),
                profile.length_mm(),
            )
            .is_some()
        {
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
            progress_mm: input.progress_mm(),
            carry_um: 0,
            speed_mm_s: input.initial_speed_mm_s(),
            length_mm: profile.length_mm(),
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
        let route_index = usize::try_from(input.route().index()).expect("route index fits usize");
        self.routes[route_index].live_vehicles += 1;
        self.live_order.push(handle);
        Ok(handle)
    }

    /// 把 live 的 Completed 车辆原子替换为新的 Active 车辆。
    ///
    /// 入口占用返回可重试的 [`ReplaceError::Blocked`]；其他失败为致命错误。
    /// 任一失败都保持已提交世界不变。成功后旧句柄立即 stale；公开契约不保证同一 slot index。
    pub fn replace_completed_vehicle(
        &mut self,
        old: VehicleHandle,
        input: VehicleSpawnInput,
    ) -> Result<VehicleReplaceRecord, ReplaceError> {
        let old_state = self.vehicle_state(old).ok_or(ReplaceError::StaleHandle)?;
        if old_state.status != VehicleStatus::Completed {
            return Err(ReplaceError::NotCompleted);
        }
        if old_state.parking.is_some() {
            return Err(ReplaceError::ParkingOccupied);
        }
        let Some(order_index) = self.live_order.iter().position(|handle| *handle == old) else {
            return Err(ReplaceError::StaleHandle);
        };

        let profile = self
            .revision
            .traffic()
            .relations()
            .vehicle_profile(input.profile())
            .ok_or(ReplaceError::UnknownProfile)?;
        let cursor = usize::try_from(input.route_edge_index()).expect("route index fits usize");
        let edge = {
            let edges = self
                .route_edges(input.route())
                .ok_or(ReplaceError::UnknownRoute)?;
            *edges
                .get(cursor)
                .ok_or(ReplaceError::RouteIndexOutOfRange)?
        };
        let length_mm = self.revision.traffic().lane_lengths_millimetres()[edge.index()];
        if input.progress_mm() > length_mm {
            return Err(ReplaceError::InvalidProgress);
        }
        let speed_limit = self
            .revision
            .traffic()
            .lane_speed_limits_millimetres_per_second()[edge.index()];
        if input.initial_speed_mm_s() > speed_limit {
            return Err(ReplaceError::SpeedExceedsLimit);
        }
        if self.route_suffix_denied(input.route(), profile.class(), cursor) {
            return Err(ReplaceError::AccessDenied);
        }
        if let Some(blocker) = self.overlap_blocker(
            input.route(),
            cursor,
            input.progress_mm(),
            profile.length_mm(),
        ) {
            let (blocker_ahead, bumper_gap) = self.overlap_relation(
                input.route(),
                cursor,
                input.progress_mm(),
                profile.length_mm(),
                blocker,
            );
            return Err(ReplaceError::Blocked(VehicleReplaceBlock {
                old,
                blocker,
                blocker_ahead,
                bumper_gap,
            }));
        }

        let old_route = old_state.route;
        let old_index = usize::try_from(old.index()).expect("vehicle index fits usize");
        let reusable_generation = self.vehicles[old_index].generation.checked_add(1);
        let slot_index = reusable_generation.map_or_else(
            || self.free_vehicles.pop().unwrap_or(self.vehicles.len()),
            |_| old_index,
        );
        let generation = reusable_generation.unwrap_or_else(|| {
            self.vehicles
                .get(slot_index)
                .map_or(0, |slot| slot.generation)
        });
        let new = VehicleHandle::new(
            u32::try_from(slot_index).expect("vehicle index fits u32"),
            generation,
        );
        let state = VehicleState {
            handle: new,
            profile: input.profile(),
            class: profile.class(),
            route: input.route(),
            route_edge_index: input.route_edge_index(),
            progress_mm: input.progress_mm(),
            carry_um: 0,
            speed_mm_s: input.initial_speed_mm_s(),
            length_mm: profile.length_mm(),
            status: VehicleStatus::Active,
            parking: None,
        };

        if reusable_generation.is_some() {
            self.vehicles[old_index] = VehicleSlot {
                generation,
                state: Some(state),
            };
        } else {
            self.vehicles[old_index].state = None;
            let slot = VehicleSlot {
                generation,
                state: Some(state),
            };
            if slot_index == self.vehicles.len() {
                self.vehicles.push(slot);
            } else {
                self.vehicles[slot_index] = slot;
            }
        }
        self.release_route_ref(old_route);
        let route_index = usize::try_from(input.route().index()).expect("route index fits usize");
        self.routes[route_index].live_vehicles += 1;
        self.live_order[order_index] = new;
        Ok(VehicleReplaceRecord { old, new })
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
        let slot_index = usize::try_from(vehicle.index()).expect("vehicle index fits usize");
        let state = self.vehicles[slot_index]
            .state
            .as_mut()
            .expect("resolved vehicle remains live");
        state.status = VehicleStatus::Parked;
        state.speed_mm_s = 0;
        state.carry_um = 0;
        state.parking = Some(space);
        self.parking_occupants[space_index] = Some(vehicle);
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
                            progress_mm: state.progress_mm,
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

    /// 已提交车辆快照。`Completed` 仍可读；stale 句柄返回 `None`。
    #[must_use]
    pub fn vehicle(&self, handle: VehicleHandle) -> Option<VehicleState> {
        self.vehicle_state(handle).copied()
    }

    /// 稳定更新顺序，含 Active / Parked / Completed。
    #[must_use]
    pub fn live_vehicles(&self) -> &[VehicleHandle] {
        &self.live_order
    }

    pub(crate) fn vehicle_state(&self, handle: VehicleHandle) -> Option<&VehicleState> {
        let slot = self.vehicles.get(usize::try_from(handle.index()).ok()?)?;
        if slot.generation != handle.generation() {
            return None;
        }
        slot.state.as_ref()
    }

    /// 本世界已注册路线的边序列。句柄无效时返回 `None`。
    #[must_use]
    pub fn route_edges(
        &self,
        route: RouteHandle,
    ) -> Option<&[laneflow_static_contract::LaneEdgeOrdinal]> {
        Some(self.compiled_route(route)?.edges.as_ref())
    }

    /// 本世界当前有效路线句柄，按槽位下标。
    pub fn live_routes(&self) -> impl Iterator<Item = RouteHandle> + '_ {
        self.routes.iter().enumerate().filter_map(|(index, slot)| {
            slot.compiled.as_ref().map(|_| {
                RouteHandle::new(
                    u32::try_from(index).expect("route index fits u32"),
                    slot.generation,
                )
            })
        })
    }

    pub(crate) fn compiled_route(&self, route: RouteHandle) -> Option<&CompiledRoute> {
        let slot = self.routes.get(usize::try_from(route.index()).ok()?)?;
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
        let Some(compiled) = self.compiled_route(route) else {
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

    fn overlap_blocker(
        &self,
        route: RouteHandle,
        cursor: usize,
        progress: u32,
        length: u32,
    ) -> Option<VehicleHandle> {
        let spawn_edges = self.route_edges(route)?;
        let lengths = self.revision.traffic().lane_lengths_millimetres();
        self.live_order.iter().copied().find(|&handle| {
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
                state.progress_mm,
                state.length_mm,
            )
        })
    }

    fn overlap_relation(
        &self,
        route: RouteHandle,
        cursor: usize,
        progress: u32,
        length: u32,
        blocker: VehicleHandle,
    ) -> (bool, i64) {
        let Some(spawn_edges) = self.route_edges(route) else {
            return (true, 0);
        };
        let Some(leader) = self.vehicle_state(blocker) else {
            return (true, 0);
        };
        let Some(blocker_edges) = self.route_edges(leader.route) else {
            return (true, 0);
        };
        let Ok(blocker_index) = usize::try_from(leader.route_edge_index) else {
            return (true, 0);
        };
        let lengths = self.revision.traffic().lane_lengths_millimetres();
        if let Some(gap) = occupancy_front_gap(
            lengths,
            spawn_edges,
            cursor,
            progress,
            blocker_edges,
            blocker_index,
            leader.progress_mm,
            leader.length_mm,
        ) {
            return (true, gap);
        }
        if let Some(gap) = occupancy_front_gap(
            lengths,
            blocker_edges,
            blocker_index,
            leader.progress_mm,
            spawn_edges,
            cursor,
            progress,
            length,
        ) {
            return (false, gap);
        }
        (true, 0)
    }

    pub(crate) fn release_route_ref(&mut self, route: RouteHandle) {
        let Ok(index) = usize::try_from(route.index()) else {
            return;
        };
        let Some(slot) = self.routes.get_mut(index) else {
            return;
        };
        if slot.generation != route.generation() || slot.compiled.is_none() {
            return;
        }
        slot.live_vehicles = slot.live_vehicles.saturating_sub(1);
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
            if duration_ms % fixed_delta_time_ms != 0 {
                return Err(InstallError::PhaseNotMultipleOfTick);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod overflow_tests {
    use super::*;
    use laneflow_format::{FormatLimits, check_canonical_network_input};
    use laneflow_static_network::{
        SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
        build_shared_network_revision,
    };

    const FULL_SPATIAL: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfca"
    );

    fn world() -> TrafficWorld {
        let input = check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD)
            .expect("checked canonical network input");
        let revision = build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::RetainAvailable,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .expect("shared network revision");
        let origin = *revision.canonical_origin();
        TrafficWorld::install(
            revision,
            WorldConfig::new(8, 4, 1, 100),
            CommittedNetworkSource::Published {
                reference: crate::PublishedLfcaReference::new(
                    "fixture://overflow-tests",
                    origin.canonical_artifact_digest(),
                    origin.canonical_artifact_byte_length(),
                    origin.network_revision(),
                )
                .expect("non-empty fixture key"),
            },
        )
        .expect("install")
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

#[cfg(test)]
mod source_tests {
    use laneflow_format::{FormatLimits, check_canonical_network_input};
    use laneflow_static_contract::{ExactByteLength, NetworkRevisionId, Sha256Digest};
    use laneflow_static_network::{
        SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
        build_shared_network_revision,
    };

    use super::*;

    use crate::PublishedLfcaReference;

    const FULL_SPATIAL: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfca"
    );

    fn revision() -> Arc<SharedNetworkRevision> {
        let input = check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD)
            .expect("checked canonical network input");
        build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::RetainAvailable,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .expect("shared network revision")
    }

    fn reference_for(origin_revision: NetworkRevisionId) -> PublishedLfcaReference {
        PublishedLfcaReference::new(
            "asset://city/roads",
            Sha256Digest::from_bytes([9; 32]),
            ExactByteLength::new(4_096),
            origin_revision,
        )
        .expect("non-empty key")
    }

    #[test]
    fn install_binds_matching_revision() {
        let revision = revision();
        let origin = *revision.canonical_origin();
        // digest / length 与根 origin 不同（重发布语义），同修订判据只比对修订标识。
        let reference = reference_for(origin.network_revision());
        let world = TrafficWorld::install(
            revision.clone(),
            WorldConfig::new(8, 4, 1, 100),
            CommittedNetworkSource::Published { reference },
        )
        .expect("source matches installed revision");
        assert_eq!(
            world.committed_source(),
            &CommittedNetworkSource::Published {
                reference: reference_for(origin.network_revision())
            }
        );
        assert_eq!(
            world.committed_source().network_revision(),
            revision.canonical_origin().network_revision()
        );
    }

    #[test]
    fn install_rejects_revision_mismatch() {
        let revision = revision();
        let mismatched = NetworkRevisionId::from_digest(Sha256Digest::from_bytes([1; 32]));
        let error = match TrafficWorld::install(
            revision.clone(),
            WorldConfig::new(8, 4, 1, 100),
            CommittedNetworkSource::Published {
                reference: reference_for(mismatched),
            },
        ) {
            Err(error) => error,
            Ok(world) => panic!("mismatched revision must fail closed"),
        };
        assert_eq!(
            error,
            InstallError::SourceRevisionMismatch {
                source_revision: mismatched,
                installed_revision: revision.canonical_origin().network_revision(),
            }
        );
    }
}
