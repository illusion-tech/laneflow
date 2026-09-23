use std::collections::VecDeque;

use laneflow_runtime::{
    ParkingError, RouteHandle, TrafficWorld, VehicleHandle, VehicleReplaceBlock,
    VehicleReplaceRecord, VehicleSpawnInput, VehicleStatus, WorldPolicySelection,
};
use laneflow_static_contract::{LaneEdgeOrdinal, NetworkRevisionId, VehicleProfileOrdinal};
use laneflow_static_network::SharedNetworkRevision;
use thiserror::Error;

use super::{BoundCorridorCatalog, BoundPortalLane, SplitMix64};

/// prepare 产出的单车计划；`spawn_input` 需要已安装 `TrafficWorld` 与 `install_routes` 句柄。
#[derive(Clone, Debug, PartialEq)]
pub struct CorridorVehiclePlan {
    /// 车辆 profile。
    pub profile: VehicleProfileOrdinal,
    /// `BoundCorridorCatalog::route_exits` / `install_routes` 返回向量的下标。
    pub route_index: usize,
    /// 路线序列下标。
    pub route_edge_index: u32,
    /// 入口边进度（毫米）。
    pub progress_mm: u32,
    /// 准备时是 `min(desiredSpeed, edge speedLimit)`。放进成功后可以更低，
    /// 与 slot 和还没取走的 `initial_vehicles` 是同一个值。
    pub initial_speed_mm_s: u32,
    /// 产出该计划的共享路网修订。
    pub network_revision: NetworkRevisionId,
    /// 产出该计划的 catalog 世界策略选择。
    pub policy_selection: WorldPolicySelection,
}

impl CorridorVehiclePlan {
    /// 把计划变成 `TrafficWorld::spawn_vehicle` 输入。
    ///
    /// # Errors
    ///
    /// 计划的网络修订、策略选择、route 下标或已注册路线与给定 `TrafficWorld`
    /// 不一致时返回 [`CorridorPopulationError::BoundWorldCatalogMismatch`]。
    pub fn spawn_input(
        &self,
        world: &TrafficWorld,
        routes: &[RouteHandle],
    ) -> Result<VehicleSpawnInput, CorridorPopulationError> {
        if world.revision().network_revision() != self.network_revision {
            return Err(CorridorPopulationError::BoundWorldCatalogMismatch {
                detail: "计划 NetworkRevisionId 与 TrafficWorld 不一致".to_owned(),
            });
        }
        if world.policy_selection() != self.policy_selection {
            return Err(CorridorPopulationError::BoundWorldCatalogMismatch {
                detail: "计划策略与 TrafficWorld 不一致".to_owned(),
            });
        }
        let route = *routes.get(self.route_index).ok_or(
            CorridorPopulationError::BoundWorldCatalogMismatch {
                detail: "计划 route_index 超出已注册路线".to_owned(),
            },
        )?;
        if world.route_edges(route).is_none() {
            return Err(CorridorPopulationError::BoundWorldCatalogMismatch {
                detail: "TrafficWorld 缺少计划中的已注册路线".to_owned(),
            });
        }
        Ok(VehicleSpawnInput::new(
            self.profile,
            route,
            self.route_edge_index,
            self.progress_mm,
            self.initial_speed_mm_s,
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
    route_entry_speeds: Vec<u32>,
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
    route_entry_speeds: Vec<u32>,
    profile: VehicleProfileOrdinal,
    rng: SplitMix64,
    slots: Vec<LogicalSlot>,
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
    /// 当前停车约束、前方限速或前后车暂时不能接纳。host 世界不变，可稍后重试。
    Retryable,
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
    /// 初始车辆按 1 m/s 降速后仍不能放进世界。车已经全部撤掉时，速度回到调用前。
    #[error("初始车辆放不进世界：{detail}")]
    InitialSpawnRejected {
        /// 运行时拒绝原因。
        detail: String,
    },
    /// 中途放不进，而且已经放进去的车没有全部撤掉。留下的车仍是降过的速度。
    /// 不能把这次当成世界是空的再放一批。
    #[error("初始车辆没有撤干净：{detail}")]
    InitialBatchRollbackIncomplete {
        /// 撤车失败原因。
        detail: String,
    },
    /// 本次 `spawn_initial_vehicles` 注册的路线没有全部撤掉。
    #[error("初始路线没有撤干净：{detail}")]
    InitialRouteRollbackIncomplete {
        /// 撤路线失败原因。
        detail: String,
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
    /// consume 必须恰好消费上一拍之后的那一拍。
    #[error("step tick {actual} 不是上次 {previous} 的下一拍")]
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
    ///
    /// # Errors
    ///
    /// 目标车辆数不在 `50..=200` 时返回
    /// [`CorridorPopulationError::InvalidTargetVehicleCount`]。
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
    ///
    /// # Errors
    ///
    /// catalog 与共享根绑定不一致（[`CorridorPopulationError::BoundWorldCatalogMismatch`]）、
    /// 车辆 profile 不在共享根、catalog 路线缺少入口边或生成槽位不足
    /// （[`CorridorPopulationError::InsufficientSpawnSlots`]）时返回相应
    /// [`CorridorPopulationError`]；失败不创建 `TrafficWorld`。
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
        let desired_speed = view.desired_speed_mm_s();
        let route_entry_speeds = catalog
            .route_exits
            .iter()
            .map(|route| {
                let entry = *route.edges.first().ok_or(
                    CorridorPopulationError::BoundWorldCatalogMismatch {
                        detail: "catalog 路线没有入口边".to_owned(),
                    },
                )?;
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
            if catalog.route_exits[route_index].edges.first() != Some(&spawn_slot.edge) {
                return Err(CorridorPopulationError::BoundWorldCatalogMismatch {
                    detail: format!("slot {:?} 入口边不是所选路线的第一条边", spawn_slot.slot_id),
                });
            }
            let route_edge_index = 0;
            let initial_speed = normal_speed_for_edge(revision, spawn_slot.edge, desired_speed)?;
            initial_vehicles.push(CorridorVehiclePlan {
                profile,
                route_index,
                route_edge_index,
                progress_mm: spawn_slot.progress_mm,
                initial_speed_mm_s: initial_speed,
                network_revision: catalog.network_revision,
                policy_selection: catalog.policy_selection,
            });
            slots.push(PreparedLogicalSlot {
                route_index,
                route_edge_index,
                edge_progress_mm: spawn_slot.progress_mm,
                initial_speed_mm_s: initial_speed,
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

    /// 读取 slot 记下的初速。供初始人口事务测试核对三处速度。
    #[doc(hidden)]
    pub fn slot_initial_speed_mm_s(&self, index: usize) -> u32 {
        self.slots[index].initial_speed_mm_s
    }

    /// 把 slot 初速改成和计划相同。供初始人口事务测试逼出降速后再失败的回滚。
    #[doc(hidden)]
    pub fn set_slot_initial_speed_mm_s(&mut self, index: usize, speed_mm_s: u32) {
        self.slots[index].initial_speed_mm_s = speed_mm_s;
    }

    /// 把 slot 进度改成和计划相同。供初始人口事务测试把车放到停不住的位置。
    #[doc(hidden)]
    pub fn set_slot_progress_mm(&mut self, index: usize, progress_mm: u32) {
        self.slots[index].edge_progress_mm = progress_mm;
    }

    /// 借用应提交给 `spawn_vehicle` 的完整初始计划。
    pub fn initial_vehicles(&self) -> &[CorridorVehiclePlan] {
        self.initial_vehicles.as_deref().unwrap_or(&[])
    }

    /// 一次性取走完整初始计划。
    pub fn take_initial_vehicles(&mut self) -> Vec<CorridorVehiclePlan> {
        self.initial_vehicles.take().unwrap_or_default()
    }

    /// 注册路线并按计划逐辆 `spawn_vehicle`。
    ///
    /// 计划里的初速是期望速度和边限速里较低的那个。这个速度若过不了当前停车、前方降速
    /// 或前后车的这一拍检查，就按 1 m/s 往下降，直到放得进。位置和路线不改。降下来的
    /// 速度写回调用方计划、slot，以及还没取走的 `initial_vehicles`。`bind` 只拿 slot
    /// 和世界里的车对。
    ///
    /// 计划已经不在时，在注册路线之前拒绝。注册成功之后若放不进，先撤已经放进去的车；
    /// 车都撤掉才把速度改回调用前，并撤掉本次注册的路线。车还在时不改回速度，也不撤路线。
    ///
    /// # Errors
    ///
    /// 路线注册失败、计划与世界不一致，或速度降到 0 仍然放不进时，返回相应错误。
    /// 撤车或撤路线没有完成时，返回 [`CorridorPopulationError::InitialBatchRollbackIncomplete`]
    /// 或 [`CorridorPopulationError::InitialRouteRollbackIncomplete`]，不要当成世界已经空了。
    pub fn spawn_initial_vehicles(
        &mut self,
        world: &mut TrafficWorld,
    ) -> Result<(Vec<VehicleHandle>, Vec<RouteHandle>), CorridorPopulationError> {
        if self.initial_vehicles.is_none() {
            return Err(CorridorPopulationError::InitialVehicleCount {
                expected: self.slots.len(),
                actual: 0,
            });
        }
        let routes = self.install_routes(world)?;
        let mut plans = self
            .initial_vehicles
            .clone()
            .expect("计划在注册路线前已经确认还在");
        match self.admit_initial_plans(world, &routes, &mut plans) {
            Ok(vehicles) => Ok((vehicles, routes)),
            Err(error @ CorridorPopulationError::InitialBatchRollbackIncomplete { .. }) => {
                Err(error)
            }
            Err(error) => {
                self.remove_registered_routes(world, &routes)?;
                Err(error)
            }
        }
    }

    /// 把已经取走的计划放进世界。顺序必须与 `prepare` 写出的 slot 相同。
    ///
    /// 第一辆生成前核对条数、顺序、修订、策略和每个 slot 的身份。对不上就拒绝，
    /// 不改速度。预检通过也不表示第一辆一定放得进；第一辆失败时世界里还没有这批车。
    ///
    /// 放进去的速度写回这份计划、对应 slot，以及还拿着的 `initial_vehicles`。
    /// 中途放不进时按相反顺序撤车。全部撤掉才把这三处改回调用前。还留着车时保持
    /// 已经写下的降速。本函数不撤路线。
    ///
    /// # Errors
    ///
    /// 与 [`Self::spawn_initial_vehicles`] 的生成失败相同。计划对不上时返回
    /// [`CorridorPopulationError::InitialVehicleCount`]、
    /// [`CorridorPopulationError::InitialVehicleMismatch`] 或
    /// [`CorridorPopulationError::BoundWorldCatalogMismatch`]。
    /// 撤不干净时返回 [`CorridorPopulationError::InitialBatchRollbackIncomplete`]。
    pub fn admit_initial_plans(
        &mut self,
        world: &mut TrafficWorld,
        routes: &[RouteHandle],
        plans: &mut [CorridorVehiclePlan],
    ) -> Result<Vec<VehicleHandle>, CorridorPopulationError> {
        self.preflight_initial_plans(world, routes, plans)?;
        let saved_speeds: Vec<u32> = plans.iter().map(|plan| plan.initial_speed_mm_s).collect();
        let mut vehicles = Vec::new();
        if vehicles.try_reserve(plans.len()).is_err() {
            return Err(CorridorPopulationError::InitialSpawnRejected {
                detail: "初始车辆名单分配失败".to_owned(),
            });
        }
        for (index, plan) in plans.iter_mut().enumerate() {
            let route = match routes.get(plan.route_index).copied() {
                Some(route) => route,
                None => {
                    self.rollback_initial_batch(world, &mut vehicles, plans, &saved_speeds)?;
                    return Err(CorridorPopulationError::BoundWorldCatalogMismatch {
                        detail: "计划 route_index 超出已注册路线".to_owned(),
                    });
                }
            };
            let mut speed = plan.initial_speed_mm_s;
            let handle = loop {
                match world.spawn_vehicle(VehicleSpawnInput::new(
                    plan.profile,
                    route,
                    plan.route_edge_index,
                    plan.progress_mm,
                    speed,
                )) {
                    Ok(handle) => break handle,
                    Err(error) if initial_speed_can_drop(&error) && speed > 0 => {
                        speed = speed.saturating_sub(1_000);
                    }
                    Err(error) => {
                        self.rollback_initial_batch(world, &mut vehicles, plans, &saved_speeds)?;
                        return Err(CorridorPopulationError::InitialSpawnRejected {
                            detail: error.to_string(),
                        });
                    }
                }
            };
            plan.initial_speed_mm_s = speed;
            if let Some(slot) = self.slots.get_mut(index) {
                slot.initial_speed_mm_s = speed;
            }
            if let Some(stored) = self
                .initial_vehicles
                .as_mut()
                .and_then(|plans| plans.get_mut(index))
            {
                stored.initial_speed_mm_s = speed;
            }
            vehicles.push(handle);
        }
        Ok(vehicles)
    }

    fn rollback_initial_batch(
        &mut self,
        world: &mut TrafficWorld,
        vehicles: &mut Vec<VehicleHandle>,
        plans: &mut [CorridorVehiclePlan],
        saved_speeds: &[u32],
    ) -> Result<(), CorridorPopulationError> {
        while let Some(handle) = vehicles.pop() {
            match world.despawn_vehicle(handle) {
                Ok(_) => {}
                Err(ParkingError::StaleVehicle) => {}
                Err(error) => {
                    return Err(CorridorPopulationError::InitialBatchRollbackIncomplete {
                        detail: error.to_string(),
                    });
                }
            }
        }
        for (index, speed) in saved_speeds.iter().copied().enumerate() {
            if let Some(plan) = plans.get_mut(index) {
                plan.initial_speed_mm_s = speed;
            }
            if let Some(slot) = self.slots.get_mut(index) {
                slot.initial_speed_mm_s = speed;
            }
            if let Some(stored) = self
                .initial_vehicles
                .as_mut()
                .and_then(|plans| plans.get_mut(index))
            {
                stored.initial_speed_mm_s = speed;
            }
        }
        Ok(())
    }

    fn remove_registered_routes(
        &self,
        world: &mut TrafficWorld,
        routes: &[RouteHandle],
    ) -> Result<(), CorridorPopulationError> {
        for handle in routes.iter().rev().copied() {
            if let Err(error) = world.remove_route(handle) {
                return Err(CorridorPopulationError::InitialRouteRollbackIncomplete {
                    detail: error.to_string(),
                });
            }
        }
        Ok(())
    }

    fn preflight_initial_plans(
        &self,
        world: &TrafficWorld,
        routes: &[RouteHandle],
        plans: &[CorridorVehiclePlan],
    ) -> Result<(), CorridorPopulationError> {
        if plans.len() != self.slots.len() {
            return Err(CorridorPopulationError::InitialVehicleCount {
                expected: self.slots.len(),
                actual: plans.len(),
            });
        }
        if routes.len() != self.catalog.route_exits.len() {
            return Err(CorridorPopulationError::BoundWorldCatalogMismatch {
                detail: "已注册路线数与 catalog 不一致".to_owned(),
            });
        }
        for (handle, exit) in routes.iter().zip(self.catalog.route_exits.iter()) {
            let Some(edges) = world.route_edges(*handle) else {
                return Err(CorridorPopulationError::BoundWorldCatalogMismatch {
                    detail: "TrafficWorld 缺少计划中的已注册路线".to_owned(),
                });
            };
            if edges != exit.edges.as_ref() {
                return Err(CorridorPopulationError::BoundWorldCatalogMismatch {
                    detail: "已注册路线边序列与 catalog 不一致".to_owned(),
                });
            }
        }
        for (index, (plan, slot)) in plans.iter().zip(self.slots.iter()).enumerate() {
            if plan.profile != self.profile
                || plan.route_index != slot.route_index
                || plan.route_edge_index != slot.route_edge_index
                || plan.progress_mm != slot.edge_progress_mm
                || plan.initial_speed_mm_s != slot.initial_speed_mm_s
                || plan.network_revision != self.catalog.network_revision
                || plan.policy_selection != self.catalog.policy_selection
            {
                return Err(CorridorPopulationError::InitialVehicleMismatch { slot_index: index });
            }
            plan.spawn_input(world, routes)?;
        }
        Ok(())
    }

    /// 对本世界每条 catalog 路线恰好 `register_route` 一次。
    ///
    /// # Errors
    ///
    /// 由 catalog `install_routes` 承接：其全部 `BindError`（含世界策略不匹配、
    /// 修订不一致与注册失败）统一包装为
    /// [`CorridorPopulationError::BoundWorldCatalogMismatch`]，仅保留诊断字符串、
    /// 变体身份丢失。
    pub fn install_routes(
        &self,
        world: &mut TrafficWorld,
    ) -> Result<Vec<RouteHandle>, CorridorPopulationError> {
        self.catalog.install_routes(world).map_err(|error| {
            CorridorPopulationError::BoundWorldCatalogMismatch {
                detail: error.to_string(),
            }
        })
    }

    /// 在 tick-0 world 上回查 identity 并进入 Running。
    ///
    /// # Errors
    ///
    /// 世界已步进（[`CorridorPopulationError::WorldAlreadyStepped`]）、绑定上下文、
    /// 初始车辆数/车辆状态与计划不一致或初始句柄重复（
    /// `DuplicateInitialVehicleHandle`）时返回相应 [`CorridorPopulationError`]；
    /// 失败不进入 Running。
    pub fn bind(
        self,
        world: &mut TrafficWorld,
        vehicles: &[VehicleHandle],
        route_handles: &[RouteHandle],
    ) -> Result<CorridorPopulationController, CorridorPopulationError> {
        if world.tick_index() != 0 {
            return Err(CorridorPopulationError::WorldAlreadyStepped {
                tick_index: world.tick_index(),
            });
        }
        if world.policy_selection() != self.catalog.policy_selection {
            return Err(CorridorPopulationError::BoundWorldCatalogMismatch {
                detail: "TrafficWorld 策略与 catalog bind 不一致".to_owned(),
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

        if route_handles.len() != self.catalog.route_exits.len() {
            return Err(CorridorPopulationError::BoundWorldCatalogMismatch {
                detail: "已注册路线数与 catalog 不一致".to_owned(),
            });
        }
        for (handle, exit) in route_handles.iter().zip(self.catalog.route_exits.iter()) {
            let Some(edges) = world.route_edges(*handle) else {
                return Err(CorridorPopulationError::BoundWorldCatalogMismatch {
                    detail: "TrafficWorld 缺少计划中的已注册路线".to_owned(),
                });
            };
            if edges != exit.edges.as_ref() {
                return Err(CorridorPopulationError::BoundWorldCatalogMismatch {
                    detail: "已注册路线边序列与 catalog 不一致".to_owned(),
                });
            }
        }
        let route_handles = route_handles.to_vec();
        let mut route_completion = Vec::with_capacity(self.catalog.route_exits.len());
        for route in &self.catalog.route_exits {
            if route.edges.is_empty() {
                return Err(CorridorPopulationError::BoundWorldCatalogMismatch {
                    detail: "catalog 路线没有边".to_owned(),
                });
            }
            route_completion.push(RouteCompletionIdentity {
                route_edge_index: u32::try_from(route.edges.len() - 1)
                    .expect("route index fits u32"),
            });
        }

        let target = self.config.target_vehicle_count;
        let mut slots = Vec::with_capacity(target);
        let mut seen = Vec::with_capacity(target);
        for (slot_index, (prepared, vehicle)) in self.slots.iter().zip(vehicles.iter()).enumerate()
        {
            let state = world
                .vehicle(*vehicle)
                .ok_or(CorridorPopulationError::InitialVehicleMismatch { slot_index })?;
            let expected_route = route_handles[prepared.route_index];
            if state.profile() != self.profile
                || state.route() != expected_route
                || state.route_edge_index() != prepared.route_edge_index
                || state.progress_mm() != prepared.edge_progress_mm
                || state.speed_mm_s() != prepared.initial_speed_mm_s
                || state.status() != VehicleStatus::Active
            {
                return Err(CorridorPopulationError::InitialVehicleMismatch { slot_index });
            }
            if seen.contains(vehicle) {
                return Err(CorridorPopulationError::DuplicateInitialVehicleHandle {
                    vehicle: *vehicle,
                });
            }
            seen.push(*vehicle);
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
            vehicle_slots: self.slots.capacity(),
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

    /// pending FIFO 中的旧句柄，队首先被 `apply_pending` 尝试。
    #[must_use]
    pub fn pending_vehicles(&self) -> Vec<VehicleHandle> {
        self.pending
            .iter()
            .map(|&slot_index| match self.slots[slot_index].state {
                LogicalSlotState::Pending { old, .. } => old,
                LogicalSlotState::Running { .. } => {
                    unreachable!("pending FIFO must only contain Pending slots")
                }
            })
            .collect()
    }

    /// 构造指定 pending slot 的替换输入。
    ///
    /// # Errors
    ///
    /// 绑定上下文与给定世界不一致、旧句柄没有对应 pending slot（
    /// [`CorridorPopulationError::UnknownCompletionVehicle`]）或替换输入构造失败时
    /// 返回相应 [`CorridorPopulationError`]。
    pub fn pending_spawn_input(
        &self,
        world: &TrafficWorld,
        old: VehicleHandle,
    ) -> Result<VehicleSpawnInput, CorridorPopulationError> {
        self.require_bound_world(world)?;
        let slot_index = self.vehicle_pending_slot(old)?;
        self.spawn_input_for_pending(world, slot_index)
    }

    /// 在一个 lifecycle boundary 内按 FIFO 各尝试一次既有 pending plan。
    ///
    /// `network_revision` 与 `policy_selection` 必须取自即将提交替换的世界，
    /// 并与 catalog bind 一致；校验失败不调用 callback、不修改 pending 状态。
    /// host callback 仍是 transport-neutral，可把同一输入交给 `TrafficWorld`
    /// 或 Adapter typed replace。宿主负责把 controller、上下文与 callback
    /// 绑定到同一个世界；这些可复制值不表示世界实例身份。
    ///
    /// # Errors
    ///
    /// 前置校验失败返回 [`CorridorReplaceApplyError::Policy`]，不调用 callback、
    /// 不修改 pending 状态；callback 返回的 outcome 与计划不符（`Replaced` 的
    /// `old` 句柄不一致或新句柄已被跟踪——宿主世界已被原子修改、该条与同界先前
    /// 的替换都不会回滚；`Blocked` 携带的 `old` 不符——宿主世界按 `Blocked`
    /// 契约保持不变）同样以 `Policy` 返回，但发生在 callback 之后；host callback
    /// 致命失败返回 [`CorridorReplaceApplyError::Host`]，当前 plan 回到 pending
    /// 队首。
    pub fn apply_pending<F, E>(
        &mut self,
        network_revision: NetworkRevisionId,
        policy_selection: WorldPolicySelection,
        mut apply: F,
    ) -> Result<CorridorBoundaryReport, CorridorReplaceApplyError<E>>
    where
        F: FnMut(VehicleHandle, VehicleSpawnInput) -> Result<CorridorReplaceAttemptOutcome, E>,
    {
        if let Err(error) = self.require_bound_context(network_revision, policy_selection) {
            return Err(CorridorReplaceApplyError::Policy(error));
        }
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
            let input = match self.spawn_input_from_plan(plan) {
                Ok(input) => input,
                Err(error) => {
                    self.pending.push_front(slot_index);
                    return Err(CorridorReplaceApplyError::Policy(error));
                }
            };
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
                CorridorReplaceAttemptOutcome::Retryable => {
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
                    if self.slot_for_vehicle(record.new).is_some() {
                        self.pending.push_front(slot_index);
                        return Err(CorridorReplaceApplyError::Policy(
                            CorridorPopulationError::ReplacementHandleAlreadyTracked {
                                vehicle: record.new,
                            },
                        ));
                    }
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
    ///
    /// # Errors
    ///
    /// 绑定上下文与给定世界不一致（
    /// [`CorridorPopulationError::BoundWorldCatalogMismatch`]）、世界步进非单调（
    /// [`CorridorPopulationError::NonMonotonicStep`]）、被跟踪车辆（Running 或
    /// Pending 旧句柄）消失、已完成车辆重复出现、
    /// 不在跟踪集合或路线/边出现项与跟踪状态不一致时返回相应
    /// [`CorridorPopulationError`]；失败不修改跟踪状态。
    pub fn consume_world(
        &mut self,
        world: &TrafficWorld,
    ) -> Result<usize, CorridorPopulationError> {
        self.require_bound_world(world)?;
        let tick = world.tick_index();
        if Some(tick) != self.last_consumed_tick.checked_add(1) {
            return Err(CorridorPopulationError::NonMonotonicStep {
                previous: self.last_consumed_tick,
                actual: tick,
            });
        }
        self.reset_completion_scratch();

        if let Some(vehicle) = self.slots.iter().find_map(|slot| {
            let handle = match slot.state {
                LogicalSlotState::Running { vehicle, .. } => vehicle,
                LogicalSlotState::Pending { old, .. } => old,
            };
            world.vehicle(handle).is_none().then_some(handle)
        }) {
            self.reset_completion_scratch();
            return Err(CorridorPopulationError::VehicleVanished { vehicle });
        }

        for handle in world.live_vehicles() {
            let Some(slot_index) = self.running_slot(*handle) else {
                let Some(state) = world.vehicle(*handle) else {
                    continue;
                };
                if state.status() == VehicleStatus::Completed
                    && self.slot_for_vehicle(*handle).is_none()
                {
                    self.reset_completion_scratch();
                    return Err(CorridorPopulationError::UnknownCompletionVehicle {
                        vehicle: *handle,
                    });
                }
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

        self.slots[slot_index].state = LogicalSlotState::Pending {
            old: vehicle,
            plan: FrozenPlan {
                route_index: target_route_index,
                portal_lane_index: target_lane_index,
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

    fn require_bound_world(&self, world: &TrafficWorld) -> Result<(), CorridorPopulationError> {
        self.require_bound_context(
            world.revision().network_revision(),
            world.policy_selection(),
        )
    }

    fn require_bound_context(
        &self,
        network_revision: NetworkRevisionId,
        policy_selection: WorldPolicySelection,
    ) -> Result<(), CorridorPopulationError> {
        if network_revision != self.catalog.network_revision {
            return Err(CorridorPopulationError::BoundWorldCatalogMismatch {
                detail: "TrafficWorld 修订与 catalog bind 不一致".to_owned(),
            });
        }
        if policy_selection != self.catalog.policy_selection {
            return Err(CorridorPopulationError::BoundWorldCatalogMismatch {
                detail: "TrafficWorld 策略与 catalog bind 不一致".to_owned(),
            });
        }
        Ok(())
    }

    fn vehicle_pending_slot(&self, old: VehicleHandle) -> Result<usize, CorridorPopulationError> {
        self.slots
            .iter()
            .position(|slot| matches!(slot.state, LogicalSlotState::Pending { old: pending, .. } if pending == old))
            .ok_or(CorridorPopulationError::UnknownCompletionVehicle { vehicle: old })
    }

    fn spawn_input_for_pending(
        &self,
        world: &TrafficWorld,
        slot_index: usize,
    ) -> Result<VehicleSpawnInput, CorridorPopulationError> {
        self.require_bound_world(world)?;
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
        let lane = self
            .catalog
            .portal_lanes
            .get(plan.portal_lane_index)
            .ok_or(CorridorPopulationError::BoundWorldCatalogMismatch {
                detail: "frozen plan portal lane 越界".to_owned(),
            })?;
        let entry = &self.catalog.spawn_slots[lane.entry_slot_index];
        Ok(VehicleSpawnInput::new(
            self.profile,
            route,
            0,
            entry.progress_mm,
            self.route_entry_speeds[plan.route_index],
        ))
    }

    fn running_slot(&self, handle: VehicleHandle) -> Option<usize> {
        self.slots.iter().position(|slot| {
            matches!(slot.state, LogicalSlotState::Running { vehicle, .. } if vehicle == handle)
        })
    }

    fn slot_for_vehicle(&self, handle: VehicleHandle) -> Option<usize> {
        self.slots.iter().position(|slot| match slot.state {
            LogicalSlotState::Running { vehicle, .. } => vehicle == handle,
            LogicalSlotState::Pending { old, .. } => old == handle,
        })
    }
}

impl CorridorReplaceAttemptOutcome {
    /// 把 Runtime replace 结果映射为 policy outcome；致命错误原样返回。
    ///
    /// # Errors
    ///
    /// Runtime 替换的致命错误原样返回（`Err`）。[`ReplaceError::Blocked`] 与当前
    /// 暂时不能接纳的运动安全错误映射为可重试 outcome，而不是错误。
    pub fn from_replace(
        result: Result<VehicleReplaceRecord, laneflow_runtime::ReplaceError>,
    ) -> Result<Self, laneflow_runtime::ReplaceError> {
        match result {
            Ok(record) => Ok(Self::Replaced(record)),
            Err(laneflow_runtime::ReplaceError::Blocked(block)) => Ok(Self::Blocked(block)),
            Err(
                laneflow_runtime::ReplaceError::StopConstraintUnsatisfiable
                | laneflow_runtime::ReplaceError::DownstreamSpeedUnsatisfiable
                | laneflow_runtime::ReplaceError::UnsafeLeader { .. }
                | laneflow_runtime::ReplaceError::UnsafeFollower { .. },
            ) => Ok(Self::Retryable),
            Err(error) => Err(error),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct PreparedLogicalSlot {
    route_index: usize,
    route_edge_index: u32,
    edge_progress_mm: u32,
    initial_speed_mm_s: u32,
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
    portal_lane_index: usize,
}

#[derive(Clone, Copy, Debug)]
struct RouteCompletionIdentity {
    route_edge_index: u32,
}

fn initial_speed_can_drop(error: &laneflow_runtime::SpawnError) -> bool {
    matches!(
        error,
        laneflow_runtime::SpawnError::StopConstraintUnsatisfiable
            | laneflow_runtime::SpawnError::DownstreamSpeedUnsatisfiable
            | laneflow_runtime::SpawnError::UnsafeLeader { .. }
            | laneflow_runtime::SpawnError::UnsafeFollower { .. }
    )
}

fn normal_speed_for_edge(
    revision: &SharedNetworkRevision,
    edge: LaneEdgeOrdinal,
    desired_speed_mm_s: u32,
) -> Result<u32, CorridorPopulationError> {
    let limits = revision
        .traffic()
        .lane_speed_limits_millimetres_per_second();
    let speed_limit = *limits
        .get(edge.index())
        .ok_or(CorridorPopulationError::MissingSpeedLimit)?;
    Ok(desired_speed_mm_s.min(speed_limit))
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
