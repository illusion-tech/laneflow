use std::collections::{HashMap, VecDeque};

use laneflow_runtime::{
    RouteHandle, TrafficWorld, VehicleHandle, VehicleReplaceBlock, VehicleReplaceRecord,
    VehicleSpawnInput, VehicleStatus,
};
use laneflow_static_contract::{LaneEdgeOrdinal, StaticRouteOrdinal, VehicleProfileOrdinal};
use laneflow_static_network::SharedNetworkRevision;
use thiserror::Error;

use super::{BoundCorridorCatalog, BoundPortalLane, SplitMix64};

/// prepare 产出的单车计划；`spawn_input` 需要已安装 `TrafficWorld`。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CorridorVehiclePlan {
    /// 车辆 profile。
    pub profile: VehicleProfileOrdinal,
    /// 共享根静态路线。
    pub route: StaticRouteOrdinal,
    /// 路线序列下标。
    pub route_edge_index: u32,
    /// 入口边进度。
    pub progress: f64,
    /// `min(desiredSpeed, edge speedLimit)`。
    pub initial_speed: f64,
}

impl CorridorVehiclePlan {
    /// 把计划变成 `TrafficWorld::spawn_vehicle` 输入。
    pub fn spawn_input(
        self,
        world: &TrafficWorld,
    ) -> Result<VehicleSpawnInput, CorridorPopulationError> {
        let route = world.static_route(self.route).map_err(|_| {
            CorridorPopulationError::BoundWorldCatalogMismatch {
                detail: "TrafficWorld 缺少计划中的静态路线".to_owned(),
            }
        })?;
        Ok(VehicleSpawnInput::new(
            self.profile,
            route,
            self.route_edge_index,
            self.progress,
            self.initial_speed,
        ))
    }
}

/// 走廊最小人口。
pub const MIN_TARGET_VEHICLE_COUNT: usize = 50;
/// 走廊最大人口。
pub const MAX_TARGET_VEHICLE_COUNT: usize = 200;
/// 走廊默认人口。
pub const DEFAULT_TARGET_VEHICLE_COUNT: usize = 100;
/// 走廊默认 replay seed。
pub const DEFAULT_SEED: u64 = 0;

/// 走廊人口配置。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CorridorPopulationConfig {
    target_vehicle_count: usize,
    seed: u64,
}

/// 两阶段启动的 prepare 结果。
#[derive(Debug)]
pub struct CorridorPopulationPrepare {
    config: CorridorPopulationConfig,
    catalog: BoundCorridorCatalog,
    profile: VehicleProfileOrdinal,
    route_entry_speeds: Vec<f64>,
    rng: SplitMix64,
    slots: Vec<PreparedLogicalSlot>,
    initial_vehicles: Option<Vec<CorridorVehiclePlan>>,
}

/// caller-owned 走廊人口 controller。
#[derive(Debug)]
pub struct CorridorPopulationController {
    catalog: BoundCorridorCatalog,
    route_handles: Vec<RouteHandle>,
    route_completion: Vec<RouteCompletionIdentity>,
    route_entry_speeds: Vec<f64>,
    profile: VehicleProfileOrdinal,
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

/// 当前 logical counts。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CorridorPopulationCounts {
    /// Running logical slots。
    pub running: usize,
    /// Pending logical slots。
    pub pending: usize,
    /// 固定目标人口。
    pub target: usize,
}

/// 有界容器 capacity，用于证明 retained state 不增长。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CorridorPopulationCapacities {
    /// logical slot table。
    pub slots: usize,
    /// handle lookup。
    pub vehicle_slots: usize,
    /// pending FIFO。
    pub pending: usize,
    /// completion scratch。
    pub completion_slots: usize,
    /// completion seen scratch。
    pub completion_seen: usize,
}

/// 单个 lifecycle boundary 的尝试统计。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CorridorBoundaryReport {
    /// 实际调用 host transaction 的次数。
    pub attempted: usize,
    /// 成功 replacement。
    pub replaced: usize,
    /// 可恢复 blocked。
    pub blocked: usize,
}

/// host replace 映射到 policy 的结果。
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum CorridorReplaceAttemptOutcome {
    /// old/new 已由 host 原子提交。
    Replaced(VehicleReplaceRecord),
    /// 入口占用，host 世界不变。
    Blocked(VehicleReplaceBlock),
}

/// `apply_pending` 的 host 或 policy 失败。
#[derive(Debug, Error)]
pub enum CorridorReplaceApplyError<E> {
    /// host transaction 致命失败；当前 plan 回到 pending 队首。
    #[error("host replace transaction 失败：{0}")]
    Host(E),
    /// host outcome 违反 policy contract。
    #[error(transparent)]
    Policy(CorridorPopulationError),
}

/// 走廊人口启动或回流失败。
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CorridorPopulationError {
    /// 目标人口不在 `50..=200`。
    #[error("target vehicle count {actual} 不在 {min}..={max} 范围内")]
    InvalidTargetVehicleCount {
        /// 最小允许值。
        min: usize,
        /// 最大允许值。
        max: usize,
        /// 实际值。
        actual: usize,
    },
    /// 物理 slot 不足以覆盖目标人口。
    #[error("需要 {required} 个 spawn slot，实际 {actual}")]
    InsufficientSpawnSlots {
        /// 目标人口。
        required: usize,
        /// 实际 slot 数。
        actual: usize,
    },
    /// bind 结果缺少 passenger-car profile。
    #[error("bound catalog 缺少车辆 profile")]
    UnknownVehicleProfile,
    /// 入口边缺少限速。
    #[error("spawn edge 缺少 speed-limit authority")]
    MissingSpeedLimit,
    /// bind 必须发生在 tick 0。
    #[error("population bind 要求 tick_index == 0，实际 {tick_index}")]
    WorldAlreadyStepped {
        /// 实际 tick。
        tick_index: u64,
    },
    /// 提交的初始句柄数量与目标人口不一致。
    #[error("初始车辆数 {actual} 与目标 {expected} 不一致")]
    InitialVehicleCount {
        /// 目标。
        expected: usize,
        /// 实际。
        actual: usize,
    },
    /// 初始车辆状态与 prepare 不一致。
    #[error("初始车辆 identity 与 prepare 结果不一致")]
    InitialVehicleMismatch {
        /// 逻辑 slot。
        slot_index: usize,
    },
    /// 初始句柄重复。
    #[error("初始车辆句柄重复：{vehicle:?}")]
    DuplicateInitialVehicleHandle {
        /// 重复句柄。
        vehicle: VehicleHandle,
    },
    /// 已绑定 catalog 与 world 路线不一致。
    #[error("bound catalog 与 TrafficWorld 不一致：{detail}")]
    BoundWorldCatalogMismatch {
        /// 诊断。
        detail: String,
    },
    /// consume 的 tick 必须严格递增。
    #[error("step tick {actual} 未大于上次 {previous}")]
    NonMonotonicStep {
        /// 上次成功消费的 tick。
        previous: u64,
        /// 本次 tick。
        actual: u64,
    },
    /// Running 句柄在 world 中消失。禁止先消失再生成。
    #[error("Running 车辆 {vehicle:?} 已从 world 消失")]
    VehicleVanished {
        /// 消失的句柄。
        vehicle: VehicleHandle,
    },
    /// 完成车辆不属于 Running slot。
    #[error("未知完成车辆 {vehicle:?}")]
    UnknownCompletionVehicle {
        /// 句柄。
        vehicle: VehicleHandle,
    },
    /// 同一 tick 重复完成。
    #[error("完成车辆 {vehicle:?} 重复")]
    DuplicateCompletionVehicle {
        /// 句柄。
        vehicle: VehicleHandle,
    },
    /// 完成路线与 logical slot 不一致。
    #[error("完成车辆 {vehicle:?} 路线不一致")]
    CompletionRouteMismatch {
        /// 句柄。
        vehicle: VehicleHandle,
    },
    /// 完成边 occurrence 不是路线末端。
    #[error("完成车辆 {vehicle:?} 边 occurrence 不是路线末端")]
    CompletionEdgeOccurrenceMismatch {
        /// 句柄。
        vehicle: VehicleHandle,
    },
    /// host 返回的 old 句柄与 pending plan 不一致。
    #[error("replace outcome old {actual:?} 不等于 {expected:?}")]
    ReplaceOutcomeOldMismatch {
        /// pending 中的 old。
        expected: VehicleHandle,
        /// host 返回。
        actual: VehicleHandle,
    },
    /// 新句柄已被本 controller 跟踪。
    #[error("replacement 句柄 {vehicle:?} 已被跟踪")]
    ReplacementHandleAlreadyTracked {
        /// 新句柄。
        vehicle: VehicleHandle,
    },
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
    /// 创建经过 `50..=200` 校验的配置。
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

    /// 目标 logical slot 数。
    pub const fn target_vehicle_count(self) -> usize {
        self.target_vehicle_count
    }

    /// 显式 replay seed。
    pub const fn seed(self) -> u64 {
        self.seed
    }
}

impl CorridorPopulationPrepare {
    /// 规划初始人口，不创建 `TrafficWorld`。
    pub fn prepare(
        config: CorridorPopulationConfig,
        catalog: BoundCorridorCatalog,
        revision: &SharedNetworkRevision,
        profile: VehicleProfileOrdinal,
    ) -> Result<Self, CorridorPopulationError> {
        if catalog.network_revision != revision.network_revision() {
            return Err(CorridorPopulationError::BoundWorldCatalogMismatch {
                detail: "catalog NetworkRevisionId 与共享根不一致".to_owned(),
            });
        }
        let view = revision
            .traffic()
            .relations()
            .vehicle_profile(profile)
            .ok_or(CorridorPopulationError::UnknownVehicleProfile)?;
        if catalog.spawn_slots.len() < config.target_vehicle_count {
            return Err(CorridorPopulationError::InsufficientSpawnSlots {
                required: config.target_vehicle_count,
                actual: catalog.spawn_slots.len(),
            });
        }
        let desired_speed = view.desired_speed();
        let route_entry_speeds = catalog
            .route_exits
            .iter()
            .map(|route| {
                let edges = revision
                    .traffic()
                    .relations()
                    .static_route_edges(route.route)
                    .ok_or(CorridorPopulationError::BoundWorldCatalogMismatch {
                        detail: "静态路线缺少边序列".to_owned(),
                    })?;
                let entry =
                    *edges
                        .first()
                        .ok_or(CorridorPopulationError::BoundWorldCatalogMismatch {
                            detail: "静态路线没有入口边".to_owned(),
                        })?;
                normal_speed_for_edge(revision, entry, desired_speed)
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
        for spawn_slot_index in shuffled_slots.into_iter().take(config.target_vehicle_count) {
            let spawn_slot = &catalog.spawn_slots[spawn_slot_index];
            let portal_lane = &catalog.portal_lanes[spawn_slot.portal_lane_index];
            let route_index = draw_weighted_route(&mut rng, portal_lane);
            let route = catalog.route_exits[route_index].route;
            let route_edge_index = route_occurrence(
                revision,
                route,
                spawn_slot.edge,
                spawn_slot.slot_id.as_str(),
            )?;
            let initial_speed = normal_speed_for_edge(revision, spawn_slot.edge, desired_speed)?;
            initial_vehicles.push(CorridorVehiclePlan {
                profile,
                route,
                route_edge_index,
                progress: spawn_slot.progress,
                initial_speed,
            });
            slots.push(PreparedLogicalSlot {
                route_index,
                route_edge_index,
                edge_progress: spawn_slot.progress,
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

    /// 借用应提交给 `spawn_vehicle` 的完整初始计划。
    pub fn initial_vehicles(&self) -> &[CorridorVehiclePlan] {
        self.initial_vehicles.as_deref().unwrap_or(&[])
    }

    /// 一次性取走完整初始计划。
    pub fn take_initial_vehicles(&mut self) -> Vec<CorridorVehiclePlan> {
        self.initial_vehicles.take().unwrap_or_default()
    }

    /// 在 tick-0 world 上回查 identity 并进入 Running。
    pub fn bind(
        self,
        world: &TrafficWorld,
        vehicles: &[VehicleHandle],
    ) -> Result<CorridorPopulationController, CorridorPopulationError> {
        if world.tick_index() != 0 {
            return Err(CorridorPopulationError::WorldAlreadyStepped {
                tick_index: world.tick_index(),
            });
        }
        if world.revision().network_revision() != self.catalog.network_revision {
            return Err(CorridorPopulationError::BoundWorldCatalogMismatch {
                detail: "TrafficWorld 修订与 catalog bind 不一致".to_owned(),
            });
        }
        if vehicles.len() != self.slots.len() {
            return Err(CorridorPopulationError::InitialVehicleCount {
                expected: self.slots.len(),
                actual: vehicles.len(),
            });
        }

        let mut route_handles = Vec::with_capacity(self.catalog.route_exits.len());
        let mut route_completion = Vec::with_capacity(self.catalog.route_exits.len());
        for route in &self.catalog.route_exits {
            let handle = world.static_route(route.route).map_err(|_| {
                CorridorPopulationError::BoundWorldCatalogMismatch {
                    detail: "TrafficWorld 缺少 catalog 静态路线".to_owned(),
                }
            })?;
            let edges = world
                .traffic()
                .relations()
                .static_route_edges(route.route)
                .ok_or(CorridorPopulationError::BoundWorldCatalogMismatch {
                    detail: "静态路线边序列不可读".to_owned(),
                })?;
            if edges.is_empty() {
                return Err(CorridorPopulationError::BoundWorldCatalogMismatch {
                    detail: "静态路线没有边".to_owned(),
                });
            }
            route_completion.push(RouteCompletionIdentity {
                route_edge_index: u32::try_from(edges.len() - 1).expect("route index fits u32"),
            });
            route_handles.push(handle);
        }

        let target = self.config.target_vehicle_count;
        let mut slots = Vec::with_capacity(target);
        let mut vehicle_slots = HashMap::with_capacity(target);
        let prepared_inputs = self.initial_vehicles.as_deref().unwrap_or(&[]);
        for (slot_index, (prepared, vehicle)) in self.slots.iter().zip(vehicles.iter()).enumerate()
        {
            let state = world
                .vehicle(*vehicle)
                .ok_or(CorridorPopulationError::InitialVehicleMismatch { slot_index })?;
            let expected = prepared_inputs
                .get(slot_index)
                .ok_or(CorridorPopulationError::InitialVehicleMismatch { slot_index })?;
            let expected_route = world.static_route(expected.route).map_err(|_| {
                CorridorPopulationError::BoundWorldCatalogMismatch {
                    detail: "TrafficWorld 缺少初始计划静态路线".to_owned(),
                }
            })?;
            if state.profile() != self.profile
                || state.route() != expected_route
                || state.route_edge_index() != prepared.route_edge_index
                || state.progress() != prepared.edge_progress
                || state.speed() != prepared.initial_speed
                || state.status() != VehicleStatus::Active
            {
                return Err(CorridorPopulationError::InitialVehicleMismatch { slot_index });
            }
            if vehicle_slots.insert(*vehicle, slot_index).is_some() {
                return Err(CorridorPopulationError::DuplicateInitialVehicleHandle {
                    vehicle: *vehicle,
                });
            }
            slots.push(LogicalSlot {
                state: LogicalSlotState::Running {
                    vehicle: *vehicle,
                    route_index: prepared.route_index,
                },
            });
        }

        Ok(CorridorPopulationController {
            catalog: self.catalog,
            route_handles,
            route_completion,
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
    /// 当前 logical population counts。
    pub const fn counts(&self) -> CorridorPopulationCounts {
        CorridorPopulationCounts {
            running: self.running_count,
            pending: self.pending_count,
            target: self.slots.len(),
        }
    }

    /// retained container capacities。
    pub fn capacities(&self) -> CorridorPopulationCapacities {
        CorridorPopulationCapacities {
            slots: self.slots.capacity(),
            vehicle_slots: self.vehicle_slots.capacity(),
            pending: self.pending.capacity(),
            completion_slots: self.completion_slots.capacity(),
            completion_seen: self.completion_seen.capacity(),
        }
    }

    /// 当前 PRNG state。
    pub const fn rng_state(&self) -> u64 {
        self.rng.state()
    }

    /// 最后成功消费的 tick。
    pub const fn last_consumed_tick(&self) -> u64 {
        self.last_consumed_tick
    }

    /// 指定 logical slot 当前 live/Completed 句柄。
    pub fn logical_vehicle(&self, logical_index: usize) -> Option<VehicleHandle> {
        self.slots.get(logical_index).map(|slot| match slot.state {
            LogicalSlotState::Running { vehicle, .. } => vehicle,
            LogicalSlotState::Pending { old, .. } => old,
        })
    }

    /// 构造指定 pending slot 的替换输入。
    pub fn pending_spawn_input(
        &self,
        world: &TrafficWorld,
        old: VehicleHandle,
    ) -> Result<VehicleSpawnInput, CorridorPopulationError> {
        let slot_index = self.vehicle_pending_slot(old)?;
        self.spawn_input_for_pending(world, slot_index)
    }

    /// 在一个 lifecycle boundary 内按 FIFO 各尝试一次既有 pending plan。
    pub fn apply_pending<F, E>(
        &mut self,
        mut apply: F,
    ) -> Result<CorridorBoundaryReport, CorridorReplaceApplyError<E>>
    where
        F: FnMut(VehicleHandle, VehicleSpawnInput) -> Result<CorridorReplaceAttemptOutcome, E>,
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
            let input = self
                .spawn_input_from_plan(plan)
                .map_err(CorridorReplaceApplyError::Policy)?;
            report.attempted += 1;
            let outcome = match apply(old, input) {
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

    /// 消费 world 中新出现的 Completed Running 车辆并入队回流计划。
    pub fn consume_world(
        &mut self,
        world: &TrafficWorld,
    ) -> Result<usize, CorridorPopulationError> {
        let tick = world.tick_index();
        if tick <= self.last_consumed_tick {
            return Err(CorridorPopulationError::NonMonotonicStep {
                previous: self.last_consumed_tick,
                actual: tick,
            });
        }
        self.reset_completion_scratch();

        if let Some(vehicle) = self
            .vehicle_slots
            .keys()
            .copied()
            .find(|vehicle| world.vehicle(*vehicle).is_none())
        {
            self.reset_completion_scratch();
            return Err(CorridorPopulationError::VehicleVanished { vehicle });
        }

        for handle in world.live_vehicles() {
            let Some(&slot_index) = self.vehicle_slots.get(handle) else {
                continue;
            };
            let Some(state) = world.vehicle(*handle) else {
                self.reset_completion_scratch();
                return Err(CorridorPopulationError::VehicleVanished { vehicle: *handle });
            };
            if state.status() != VehicleStatus::Completed {
                continue;
            }
            if self.completion_seen[slot_index] {
                self.reset_completion_scratch();
                return Err(CorridorPopulationError::DuplicateCompletionVehicle {
                    vehicle: *handle,
                });
            }
            let LogicalSlotState::Running {
                vehicle,
                route_index,
            } = self.slots[slot_index].state
            else {
                self.reset_completion_scratch();
                return Err(CorridorPopulationError::UnknownCompletionVehicle { vehicle: *handle });
            };
            debug_assert_eq!(vehicle, *handle);
            if state.route() != self.route_handles[route_index] {
                self.reset_completion_scratch();
                return Err(CorridorPopulationError::CompletionRouteMismatch { vehicle: *handle });
            }
            let expected = self.route_completion[route_index];
            if state.route_edge_index() != expected.route_edge_index {
                self.reset_completion_scratch();
                return Err(CorridorPopulationError::CompletionEdgeOccurrenceMismatch {
                    vehicle: *handle,
                });
            }
            self.completion_seen[slot_index] = true;
            self.completion_slots.push(slot_index);
        }

        let completed = self.completion_slots.len();
        for completion_index in 0..completed {
            let slot_index = self.completion_slots[completion_index];
            self.enqueue_running_completion(slot_index);
        }
        self.last_consumed_tick = tick;
        debug_assert_eq!(self.running_count + self.pending_count, self.slots.len());
        Ok(completed)
    }

    fn enqueue_running_completion(&mut self, slot_index: usize) {
        let LogicalSlotState::Running {
            vehicle,
            route_index,
        } = self.slots[slot_index].state
        else {
            unreachable!("completion batch was validated before commit");
        };
        let exit_portal_index =
            usize::from(self.catalog.route_exits[route_index].exit_portal_index);
        let portal_draw = self.rng.uniform(5) as usize;
        let target_portal_index = if portal_draw >= exit_portal_index {
            portal_draw + 1
        } else {
            portal_draw
        };
        let target_lanes = &self.catalog.portal_lane_indices[target_portal_index];
        let lane_draw = self.rng.uniform(target_lanes.len() as u64) as usize;
        let target_lane_index = target_lanes[lane_draw];
        let target_route_index =
            draw_weighted_route(&mut self.rng, &self.catalog.portal_lanes[target_lane_index]);

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

    fn reset_completion_scratch(&mut self) {
        for slot_index in self.completion_slots.drain(..) {
            self.completion_seen[slot_index] = false;
        }
    }

    fn vehicle_pending_slot(&self, old: VehicleHandle) -> Result<usize, CorridorPopulationError> {
        self.slots
            .iter()
            .position(|slot| matches!(slot.state, LogicalSlotState::Pending { old: pending, .. } if pending == old))
            .ok_or(CorridorPopulationError::UnknownCompletionVehicle { vehicle: old })
    }

    fn spawn_input_for_pending(
        &self,
        _world: &TrafficWorld,
        slot_index: usize,
    ) -> Result<VehicleSpawnInput, CorridorPopulationError> {
        let LogicalSlotState::Pending { plan, .. } = self.slots[slot_index].state else {
            return Err(CorridorPopulationError::UnknownCompletionVehicle {
                vehicle: self.logical_vehicle(slot_index).expect("slot exists"),
            });
        };
        self.spawn_input_from_plan(plan)
    }

    fn spawn_input_from_plan(
        &self,
        plan: FrozenPlan,
    ) -> Result<VehicleSpawnInput, CorridorPopulationError> {
        let route = self.route_handles[plan.route_index];
        let entry_slot_index = self.catalog.route_exits[plan.route_index].entry_slot_index;
        let entry = &self.catalog.spawn_slots[entry_slot_index];
        Ok(VehicleSpawnInput::new(
            self.profile,
            route,
            0,
            entry.progress,
            self.route_entry_speeds[plan.route_index],
        ))
    }
}

impl CorridorReplaceAttemptOutcome {
    /// 把 Runtime replace 结果映射为 policy outcome；致命错误原样返回。
    pub fn from_replace(
        result: Result<VehicleReplaceRecord, laneflow_runtime::ReplaceError>,
    ) -> Result<Self, laneflow_runtime::ReplaceError> {
        match result {
            Ok(record) => Ok(Self::Replaced(record)),
            Err(laneflow_runtime::ReplaceError::Blocked(block)) => Ok(Self::Blocked(block)),
            Err(error) => Err(error),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct PreparedLogicalSlot {
    route_index: usize,
    route_edge_index: u32,
    edge_progress: f64,
    initial_speed: f64,
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
    route_edge_index: u32,
}

fn normal_speed_for_edge(
    revision: &SharedNetworkRevision,
    edge: LaneEdgeOrdinal,
    desired_speed: f64,
) -> Result<f64, CorridorPopulationError> {
    let limits = revision.traffic().lane_speed_limits_meters_per_second();
    let speed_limit = *limits
        .get(edge.index())
        .ok_or(CorridorPopulationError::MissingSpeedLimit)?;
    if !speed_limit.is_finite() {
        return Err(CorridorPopulationError::MissingSpeedLimit);
    }
    Ok(desired_speed.min(speed_limit).max(0.0))
}

fn route_occurrence(
    revision: &SharedNetworkRevision,
    route: StaticRouteOrdinal,
    edge: LaneEdgeOrdinal,
    slot_id: &str,
) -> Result<u32, CorridorPopulationError> {
    let edges = revision
        .traffic()
        .relations()
        .static_route_edges(route)
        .ok_or_else(|| CorridorPopulationError::BoundWorldCatalogMismatch {
            detail: format!("slot {slot_id:?} route 不可读"),
        })?;
    let index = edges
        .iter()
        .position(|candidate| *candidate == edge)
        .ok_or_else(|| CorridorPopulationError::BoundWorldCatalogMismatch {
            detail: format!("slot {slot_id:?} edge 不在所选 route 上"),
        })?;
    u32::try_from(index).map_err(|_| CorridorPopulationError::BoundWorldCatalogMismatch {
        detail: format!("slot {slot_id:?} route edge index 溢出"),
    })
}

fn draw_weighted_route(rng: &mut SplitMix64, lane: &BoundPortalLane) -> usize {
    let mut draw = rng.uniform(lane.total_positive_weight);
    for choice in &lane.choices {
        if draw < choice.weight {
            return choice.route_index;
        }
        draw -= choice.weight;
    }
    unreachable!("normalized route-choice weights cover the complete draw range")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signalized_corridor::BoundRouteChoice;

    fn lane(choices: &[(usize, u64)]) -> BoundPortalLane {
        BoundPortalLane {
            portal_index: 0,
            lane_index: 0,
            entry_slot_index: 0,
            choices: choices
                .iter()
                .map(|(route_index, weight)| BoundRouteChoice {
                    route_index: *route_index,
                    weight: *weight,
                })
                .collect(),
            total_positive_weight: choices.iter().map(|(_, weight)| *weight).sum(),
        }
    }

    #[test]
    fn single_route_choice_still_consumes_its_raw_weight_draw() {
        let mut rng = SplitMix64::new(7);
        let before = rng.state();
        assert_eq!(draw_weighted_route(&mut rng, &lane(&[(3, 20)])), 3);
        assert_ne!(rng.state(), before);
    }

    #[test]
    fn weighted_route_choice_uses_frozen_cumulative_order() {
        let mut rng = SplitMix64::new(7);
        let lane = lane(&[(7, 2), (9, 3)]);
        assert_eq!(
            (0..4)
                .map(|_| draw_weighted_route(&mut rng, &lane))
                .collect::<Vec<_>>(),
            [9, 9, 7, 9]
        );
    }
}
