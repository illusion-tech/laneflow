use std::collections::{HashMap, VecDeque};

use laneflow_core::{
    CoreEvent, CoreWorld, EdgeHandle, InitialTrafficData, RouteHandle, Speed, StepResult,
    VehicleHandle, VehicleProfileHandle, VehicleReplaceBlock, VehicleReplaceExternalId,
    VehicleReplaceInput, VehicleReplaceOutcome, VehicleReplaceRecord, VehicleSpawnInput,
    VehicleStatus,
};

use super::{CorridorPopulationError, NormalizedCorridorCatalog, SplitMix64};

/// v0.8 corridor 最小人口。
pub const MIN_TARGET_VEHICLE_COUNT: usize = 50;
/// v0.8 corridor 最大人口。
pub const MAX_TARGET_VEHICLE_COUNT: usize = 200;
/// v0.8 corridor 默认人口。
pub const DEFAULT_TARGET_VEHICLE_COUNT: usize = 100;
/// v0.8 corridor 默认 replay seed。
pub const DEFAULT_SEED: u64 = 0;

/// v0.8 signalized-corridor population domain config。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CorridorPopulationConfig {
    target_vehicle_count: usize,
    seed: u64,
}

/// 两阶段 bootstrap 的 prepare 结果。
#[derive(Debug)]
pub struct CorridorPopulationPrepare {
    config: CorridorPopulationConfig,
    catalog: NormalizedCorridorCatalog,
    profile: VehicleProfileHandle,
    route_entry_speeds: Vec<Speed>,
    rng: SplitMix64,
    slots: Vec<PreparedLogicalSlot>,
    initial_vehicles: Option<Vec<VehicleSpawnInput>>,
}

/// caller-owned corridor logical population controller。
#[derive(Debug)]
pub struct CorridorPopulationController {
    catalog: NormalizedCorridorCatalog,
    route_handles: Vec<RouteHandle>,
    route_completion_edges: Vec<RouteCompletionIdentity>,
    route_entry_speeds: Vec<Speed>,
    profile: VehicleProfileHandle,
    rng: SplitMix64,
    slots: Vec<LogicalSlot>,
    vehicle_slots: HashMap<VehicleHandle, usize>,
    pending: VecDeque<usize>,
    completion_slots: Vec<usize>,
    completion_seen: Vec<bool>,
    running_count: usize,
    pending_count: usize,
    last_consumed_tick: u64,
}

/// controller 当前 logical counts。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CorridorPopulationCounts {
    /// Running logical slots。
    pub running: usize,
    /// Pending logical slots。
    pub pending: usize,
    /// 固定目标人口。
    pub target: usize,
}

/// 用于证明 steady retained state 有界的容器 capacities。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CorridorPopulationCapacities {
    /// logical slot table capacity。
    pub slots: usize,
    /// active handle lookup capacity。
    pub vehicle_slots: usize,
    /// pending FIFO capacity。
    pub pending: usize,
    /// completion validation scratch capacity。
    pub completion_slots: usize,
    /// completion seen scratch capacity。
    pub completion_seen: usize,
}

/// 单个 lifecycle boundary 的尝试统计。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CorridorBoundaryReport {
    /// 本 boundary 实际调用 host transaction 的数量。
    pub attempted: usize,
    /// 成功 replacement 数量。
    pub replaced: usize,
    /// 可恢复 blocked 数量。
    pub blocked: usize,
}

/// Core/Adapter transaction 映射到 policy 的 engine-neutral outcome。
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum CorridorReplaceAttemptOutcome {
    /// old/new identity 已由 host transaction 原子提交。
    Replaced(VehicleReplaceRecord),
    /// 入口仍被占用，host authority 未变化。
    Blocked(VehicleReplaceBlock),
}

/// `apply_pending` 的 host failure 或 policy contract failure。
#[derive(Debug, thiserror::Error)]
pub enum CorridorReplaceApplyError<E> {
    /// host transaction 返回 fatal error；当前 plan 已恢复到 pending 队首。
    #[error("host replace transaction 失败：{0}")]
    Host(E),
    /// host outcome 违反 closure contract。
    #[error(transparent)]
    Policy(CorridorPopulationError),
}

impl Default for CorridorPopulationConfig {
    fn default() -> Self {
        Self {
            target_vehicle_count: DEFAULT_TARGET_VEHICLE_COUNT,
            seed: DEFAULT_SEED,
        }
    }
}

impl CorridorPopulationConfig {
    /// 创建经过 `50..=200` 校验的 corridor config。
    pub fn try_new(
        target_vehicle_count: usize,
        seed: u64,
    ) -> Result<Self, CorridorPopulationError> {
        if !(MIN_TARGET_VEHICLE_COUNT..=MAX_TARGET_VEHICLE_COUNT).contains(&target_vehicle_count) {
            return Err(CorridorPopulationError::InvalidTargetVehicleCount {
                min: MIN_TARGET_VEHICLE_COUNT,
                max: MAX_TARGET_VEHICLE_COUNT,
                actual: target_vehicle_count,
            });
        }
        Ok(Self {
            target_vehicle_count,
            seed,
        })
    }

    /// 返回目标 logical slot 数量。
    pub const fn target_vehicle_count(self) -> usize {
        self.target_vehicle_count
    }

    /// 返回显式 replay seed。
    pub const fn seed(self) -> u64 {
        self.seed
    }
}

impl CorridorPopulationPrepare {
    /// 规划初始 population，但不创建或持有 `CoreWorld`。
    pub fn prepare(
        config: CorridorPopulationConfig,
        catalog: NormalizedCorridorCatalog,
        traffic: &InitialTrafficData,
        profile: VehicleProfileHandle,
    ) -> Result<Self, CorridorPopulationError> {
        if traffic.vehicle_profiles().profile(profile).is_none() {
            return Err(CorridorPopulationError::UnknownVehicleProfile);
        }
        if catalog.spawn_slots.len() < config.target_vehicle_count {
            return Err(CorridorPopulationError::InsufficientSpawnSlots {
                required: config.target_vehicle_count,
                actual: catalog.spawn_slots.len(),
            });
        }
        let desired_speed = traffic
            .vehicle_profiles()
            .profile(profile)
            .expect("profile was validated above")
            .iidm()
            .desired_speed;
        let route_entry_speeds = catalog
            .routes
            .iter()
            .map(|route| {
                let entry = &catalog.spawn_slots[route.entry_spawn_slot_index];
                normal_speed_for_edge(traffic, &entry.edge_id, desired_speed)
            })
            .collect::<Result<Vec<_>, _>>()?;

        let mut rng = SplitMix64::new(config.seed);
        let mut shuffled_slots = (0..catalog.spawn_slots.len()).collect::<Vec<_>>();
        for index in (1..shuffled_slots.len()).rev() {
            let swap_index = rng.uniform((index + 1) as u64) as usize;
            shuffled_slots.swap(index, swap_index);
        }

        let mut slots = Vec::with_capacity(config.target_vehicle_count);
        let mut initial_vehicles = Vec::with_capacity(config.target_vehicle_count);
        for (logical_index, spawn_slot_index) in shuffled_slots
            .into_iter()
            .take(config.target_vehicle_count)
            .enumerate()
        {
            let spawn_slot = &catalog.spawn_slots[spawn_slot_index];
            let route = &catalog.routes[spawn_slot.route_index];
            let initial_speed = normal_speed_for_edge(traffic, &spawn_slot.edge_id, desired_speed)?;
            let vehicle_id = format!("corridor-vehicle-{logical_index:03}");
            initial_vehicles.push(VehicleSpawnInput::active(
                vehicle_id.clone(),
                profile,
                route.id.clone(),
                spawn_slot.route_edge_index,
                spawn_slot.edge_progress,
                initial_speed,
            ));
            slots.push(PreparedLogicalSlot {
                vehicle_id,
                route_index: spawn_slot.route_index,
                route_edge_index: spawn_slot.route_edge_index,
                edge_progress: spawn_slot.edge_progress,
                initial_speed,
            });
        }

        Ok(Self {
            config,
            catalog,
            profile,
            route_entry_speeds,
            rng,
            slots,
            initial_vehicles: Some(initial_vehicles),
        })
    }

    /// 借用 caller 应提交给 `CoreWorld::with_traffic_data` 的完整初始 batch。
    pub fn initial_vehicles(&self) -> &[VehicleSpawnInput] {
        self.initial_vehicles.as_deref().unwrap_or(&[])
    }

    /// 一次性取走完整初始 batch。
    pub fn take_initial_vehicles(&mut self) -> Vec<VehicleSpawnInput> {
        self.initial_vehicles.take().unwrap_or_default()
    }

    /// 在成功创建的 tick-0 world 上回查全部 identity，并进入 Running。
    pub fn bind(
        self,
        world: &CoreWorld,
    ) -> Result<CorridorPopulationController, CorridorPopulationError> {
        if world.tick_index() != 0 {
            return Err(CorridorPopulationError::WorldAlreadyStepped {
                tick_index: world.tick_index(),
            });
        }
        if world.vehicle_profile(self.profile).is_none() {
            return Err(CorridorPopulationError::UnknownVehicleProfile);
        }

        let mut route_handles = Vec::with_capacity(self.catalog.routes.len());
        let mut route_completion_edges = Vec::with_capacity(self.catalog.routes.len());
        for route in &self.catalog.routes {
            let Some(route_handle) = world.route_handle(&route.id) else {
                return Err(CorridorPopulationError::BoundWorldCatalogMismatch {
                    detail: format!("world 缺少 route {:?}", route.id),
                });
            };
            route_handles.push(route_handle);
            let route_edges = world.route_edges(route_handle).ok_or_else(|| {
                CorridorPopulationError::BoundWorldCatalogMismatch {
                    detail: format!("world route {:?} 已 stale", route.id),
                }
            })?;
            let Some(edge) = route_edges.last().copied() else {
                return Err(CorridorPopulationError::BoundWorldCatalogMismatch {
                    detail: format!("world route {:?} 没有 edge", route.id),
                });
            };
            route_completion_edges.push(RouteCompletionIdentity {
                edge,
                route_edge_index: route_edges.len() - 1,
            });
        }
        for spawn_slot in &self.catalog.spawn_slots {
            let route_handle = route_handles[spawn_slot.route_index];
            let route_edges = world.route_edges(route_handle).ok_or_else(|| {
                CorridorPopulationError::BoundWorldCatalogMismatch {
                    detail: format!(
                        "world route {:?} 已 stale",
                        self.catalog.routes[spawn_slot.route_index].id
                    ),
                }
            })?;
            let Some(edge) = route_edges.get(spawn_slot.route_edge_index) else {
                return Err(CorridorPopulationError::BoundWorldCatalogMismatch {
                    detail: format!("slot {:?} route edge index 越界", spawn_slot.id),
                });
            };
            if world.edge_external_id(*edge) != Some(spawn_slot.edge_id.as_str()) {
                return Err(CorridorPopulationError::BoundWorldCatalogMismatch {
                    detail: format!("slot {:?} edge identity 不一致", spawn_slot.id),
                });
            }
        }

        let target = self.config.target_vehicle_count;
        let mut slots = Vec::with_capacity(target);
        let mut vehicle_slots = HashMap::with_capacity(target);
        for (slot_index, prepared) in self.slots.into_iter().enumerate() {
            let vehicle = world.vehicle_handle(&prepared.vehicle_id).ok_or_else(|| {
                CorridorPopulationError::MissingInitialVehicle {
                    vehicle_id: prepared.vehicle_id.clone(),
                }
            })?;
            let state = world.vehicle(vehicle).ok_or_else(|| {
                CorridorPopulationError::MissingInitialVehicle {
                    vehicle_id: prepared.vehicle_id.clone(),
                }
            })?;
            if state.profile != self.profile
                || state.route != route_handles[prepared.route_index]
                || state.route_edge_index != prepared.route_edge_index
                || state.edge_progress != prepared.edge_progress
                || state.current_speed != prepared.initial_speed
                || state.status != VehicleStatus::Active
            {
                return Err(CorridorPopulationError::InitialVehicleMismatch {
                    vehicle_id: prepared.vehicle_id,
                });
            }
            if vehicle_slots.insert(vehicle, slot_index).is_some() {
                return Err(CorridorPopulationError::DuplicateInitialVehicleHandle { vehicle });
            }
            slots.push(LogicalSlot {
                state: LogicalSlotState::Running {
                    vehicle,
                    route_index: prepared.route_index,
                },
            });
        }

        Ok(CorridorPopulationController {
            catalog: self.catalog,
            route_handles,
            route_completion_edges,
            route_entry_speeds: self.route_entry_speeds,
            profile: self.profile,
            rng: self.rng,
            slots,
            vehicle_slots,
            pending: VecDeque::with_capacity(target),
            completion_slots: Vec::with_capacity(target),
            completion_seen: vec![false; target],
            running_count: target,
            pending_count: 0,
            last_consumed_tick: world.tick_index(),
        })
    }
}

impl CorridorPopulationController {
    /// 返回当前 logical population counts。
    pub const fn counts(&self) -> CorridorPopulationCounts {
        CorridorPopulationCounts {
            running: self.running_count,
            pending: self.pending_count,
            target: self.slots.len(),
        }
    }

    /// 返回 retained containers 的当前 capacities。
    pub fn capacities(&self) -> CorridorPopulationCapacities {
        CorridorPopulationCapacities {
            slots: self.slots.capacity(),
            vehicle_slots: self.vehicle_slots.capacity(),
            pending: self.pending.capacity(),
            completion_slots: self.completion_slots.capacity(),
            completion_seen: self.completion_seen.capacity(),
        }
    }

    /// 返回当前 PRNG state。
    pub const fn rng_state(&self) -> u64 {
        self.rng.state()
    }

    /// 返回最后成功消费的 Core tick。
    pub const fn last_consumed_tick(&self) -> u64 {
        self.last_consumed_tick
    }

    /// 返回指定 logical slot 当前 live/Completed handle。
    pub fn logical_vehicle(&self, logical_index: usize) -> Option<VehicleHandle> {
        self.slots.get(logical_index).map(|slot| match slot.state {
            LogicalSlotState::Running { vehicle, .. } => vehicle,
            LogicalSlotState::Pending { old, .. } => old,
        })
    }

    /// 在一个 lifecycle boundary 内按 FIFO 各尝试一次既有 pending plan。
    pub fn apply_pending<F, E>(
        &mut self,
        mut apply: F,
    ) -> Result<CorridorBoundaryReport, CorridorReplaceApplyError<E>>
    where
        F: FnMut(VehicleHandle, &VehicleReplaceInput) -> Result<CorridorReplaceAttemptOutcome, E>,
    {
        let boundary_pending = self.pending.len();
        let mut report = CorridorBoundaryReport::default();
        for _ in 0..boundary_pending {
            let slot_index = self
                .pending
                .pop_front()
                .expect("boundary count came from pending length");
            let LogicalSlotState::Pending { old, plan } = self.slots[slot_index].state else {
                unreachable!("pending FIFO must only contain Pending slots");
            };
            let route = &self.catalog.routes[plan.route_index];
            let entry = &self.catalog.spawn_slots[route.entry_spawn_slot_index];
            let input = VehicleReplaceInput::new(
                VehicleReplaceExternalId::Preserve,
                self.profile,
                self.route_handles[plan.route_index],
                entry.route_edge_index,
                entry.edge_progress,
                self.route_entry_speeds[plan.route_index],
            );
            report.attempted += 1;
            let outcome = match apply(old, &input) {
                Ok(outcome) => outcome,
                Err(error) => {
                    self.pending.push_front(slot_index);
                    return Err(CorridorReplaceApplyError::Host(error));
                }
            };
            match outcome {
                CorridorReplaceAttemptOutcome::Blocked(block) => {
                    if block.old != old {
                        self.pending.push_front(slot_index);
                        return Err(CorridorReplaceApplyError::Policy(
                            CorridorPopulationError::ReplaceOutcomeOldMismatch {
                                expected: old,
                                actual: block.old,
                            },
                        ));
                    }
                    self.pending.push_back(slot_index);
                    report.blocked += 1;
                }
                CorridorReplaceAttemptOutcome::Replaced(record) => {
                    if record.old != old {
                        self.pending.push_front(slot_index);
                        return Err(CorridorReplaceApplyError::Policy(
                            CorridorPopulationError::ReplaceOutcomeOldMismatch {
                                expected: old,
                                actual: record.old,
                            },
                        ));
                    }
                    if self.vehicle_slots.contains_key(&record.new) {
                        self.pending.push_front(slot_index);
                        return Err(CorridorReplaceApplyError::Policy(
                            CorridorPopulationError::ReplacementHandleAlreadyTracked {
                                vehicle: record.new,
                            },
                        ));
                    }
                    let replaced = self.vehicle_slots.insert(record.new, slot_index);
                    debug_assert!(replaced.is_none());
                    self.slots[slot_index].state = LogicalSlotState::Running {
                        vehicle: record.new,
                        route_index: plan.route_index,
                    };
                    self.running_count += 1;
                    self.pending_count -= 1;
                    report.replaced += 1;
                }
            }
        }
        debug_assert_eq!(self.running_count + self.pending_count, self.slots.len());
        Ok(report)
    }

    /// 原子验证并消费一个 ordered Core `StepResult`。
    pub fn consume_step_result(
        &mut self,
        step: &StepResult,
    ) -> Result<usize, CorridorPopulationError> {
        if step.tick_index <= self.last_consumed_tick {
            return Err(CorridorPopulationError::NonMonotonicStep {
                previous: self.last_consumed_tick,
                actual: step.tick_index,
            });
        }
        self.reset_completion_scratch();

        for event in &step.events {
            let CoreEvent::VehicleCompletedRoute(completion) = event else {
                continue;
            };
            if completion.tick_index != step.tick_index {
                self.reset_completion_scratch();
                return Err(CorridorPopulationError::CompletionTickMismatch {
                    step_tick: step.tick_index,
                    event_tick: completion.tick_index,
                });
            }
            let Some(slot_index) = self.vehicle_slots.get(&completion.vehicle).copied() else {
                self.reset_completion_scratch();
                return Err(CorridorPopulationError::UnknownCompletionVehicle {
                    vehicle: completion.vehicle,
                });
            };
            if self.completion_seen[slot_index] {
                self.reset_completion_scratch();
                return Err(CorridorPopulationError::DuplicateCompletionVehicle {
                    vehicle: completion.vehicle,
                });
            }
            let LogicalSlotState::Running {
                vehicle,
                route_index,
            } = self.slots[slot_index].state
            else {
                self.reset_completion_scratch();
                return Err(CorridorPopulationError::UnknownCompletionVehicle {
                    vehicle: completion.vehicle,
                });
            };
            debug_assert_eq!(vehicle, completion.vehicle);
            let expected_route = self.route_handles[route_index];
            if completion.route != expected_route {
                self.reset_completion_scratch();
                return Err(CorridorPopulationError::CompletionRouteMismatch {
                    vehicle: completion.vehicle,
                    expected: expected_route,
                    actual: completion.route,
                });
            }
            let expected_completion = self.route_completion_edges[route_index];
            if completion.edge != expected_completion.edge
                || completion.route_edge_index != expected_completion.route_edge_index
            {
                self.reset_completion_scratch();
                return Err(CorridorPopulationError::CompletionEdgeOccurrenceMismatch {
                    vehicle: completion.vehicle,
                    expected_edge: expected_completion.edge,
                    expected_route_edge_index: expected_completion.route_edge_index,
                    actual_edge: completion.edge,
                    actual_route_edge_index: completion.route_edge_index,
                });
            }
            self.completion_seen[slot_index] = true;
            self.completion_slots.push(slot_index);
        }

        let completed = self.completion_slots.len();
        for completion_index in 0..completed {
            let slot_index = self.completion_slots[completion_index];
            let LogicalSlotState::Running {
                vehicle,
                route_index,
            } = self.slots[slot_index].state
            else {
                unreachable!("completion batch was validated before commit");
            };
            let exit_portal_index = self.catalog.routes[route_index].exit_portal_index;
            let portal_draw = self.rng.uniform(5) as usize;
            let target_portal_index = if portal_draw >= exit_portal_index {
                portal_draw + 1
            } else {
                portal_draw
            };
            let target_routes = &self.catalog.portals[target_portal_index].route_indices;
            let lane_draw = self.rng.uniform(target_routes.len() as u64) as usize;
            let target_route_index = target_routes[lane_draw];

            let removed = self.vehicle_slots.remove(&vehicle);
            debug_assert_eq!(removed, Some(slot_index));
            self.slots[slot_index].state = LogicalSlotState::Pending {
                old: vehicle,
                plan: FrozenPlan {
                    route_index: target_route_index,
                },
            };
            self.pending.push_back(slot_index);
            self.running_count -= 1;
            self.pending_count += 1;
        }
        self.last_consumed_tick = step.tick_index;
        debug_assert_eq!(self.running_count + self.pending_count, self.slots.len());
        Ok(completed)
    }

    fn reset_completion_scratch(&mut self) {
        for slot_index in self.completion_slots.drain(..) {
            self.completion_seen[slot_index] = false;
        }
    }
}

impl CorridorReplaceAttemptOutcome {
    /// 将 lockstep `laneflow-core` outcome 映射到 scenario policy。
    pub fn from_core(outcome: VehicleReplaceOutcome) -> Self {
        match outcome {
            VehicleReplaceOutcome::Replaced(record) => Self::Replaced(record),
            VehicleReplaceOutcome::Blocked(block) => Self::Blocked(block),
            _ => unreachable!("laneflow-core and laneflow-scenario are released in lockstep"),
        }
    }
}

#[derive(Clone, Debug)]
struct PreparedLogicalSlot {
    vehicle_id: String,
    route_index: usize,
    route_edge_index: usize,
    edge_progress: laneflow_core::EdgeProgress,
    initial_speed: Speed,
}

#[derive(Clone, Copy, Debug)]
struct LogicalSlot {
    state: LogicalSlotState,
}

#[derive(Clone, Copy, Debug)]
enum LogicalSlotState {
    Running {
        vehicle: VehicleHandle,
        route_index: usize,
    },
    Pending {
        old: VehicleHandle,
        plan: FrozenPlan,
    },
}

#[derive(Clone, Copy, Debug)]
struct FrozenPlan {
    route_index: usize,
}

#[derive(Clone, Copy, Debug)]
struct RouteCompletionIdentity {
    edge: EdgeHandle,
    route_edge_index: usize,
}

fn normal_speed_for_edge(
    traffic: &InitialTrafficData,
    edge_id: &str,
    desired_speed: f64,
) -> Result<Speed, CorridorPopulationError> {
    let speed_limit = traffic
        .lane_graph()
        .edge_speed_limit_by_id(edge_id)
        .ok_or_else(|| CorridorPopulationError::BoundWorldCatalogMismatch {
            detail: format!("spawn edge {edge_id:?} 缺少 speed-limit authority"),
        })?;
    Speed::try_new(desired_speed.min(speed_limit.value())).map_err(|source| {
        CorridorPopulationError::BoundWorldCatalogMismatch {
            detail: format!("spawn edge {edge_id:?} 无法派生正常初速度：{source}"),
        }
    })
}
