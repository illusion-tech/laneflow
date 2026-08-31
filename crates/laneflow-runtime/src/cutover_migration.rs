//! 跨修订直移核心（#302 切换合同 §3；#513 切片 C-2）。
//!
//! LFSD 认证消费见 [`crate::cutover`] 的描述符方法。本模块交付另外两件：
//! base→target 稳定引用重绑表（两侧 `SharedIdentityIndex` 共同解析，LFSD
//! 绑定行完成两侧制品交叉验证后，映射权威即身份索引对）与结构克隆迁移
//! （候选与旧世界槽位布局一致——当期句柄恒等保持；逐实体重绑即重验证，
//! 任一实体引用不存在或原样重绑违反 target 不变量都整体失败关闭）。

use std::sync::Arc;

use laneflow_static_contract::{
    EntityKind, EntityKindMarker, LaneEdgeOrdinal, Ordinal, OrdinalKind, ParkingFacilityOrdinal,
    ParkingSpaceOrdinal, ParticipantClassOrdinal, SignalAspect, VehicleProfileOrdinal,
};
use laneflow_static_network::{SharedIdentityIndex, SharedNetworkRevision};

use crate::parking::ParkingRuntimeState;
use crate::tables::{RouteSlot, VehicleSlot, bodies_overlap, compile_route, route_access_denied};
use crate::{
    CommittedNetworkSource, CutoverError, ParkingBinding, ParkingReservation, ParkingSpaceState,
    ParkingTarget, TrafficWorld, VehicleHandle, VehicleStatus, VirtualEntryAnchorSelector,
};

#[cfg(test)]
thread_local! {
    static STAGING_RESERVATIONS_BEFORE_FAILURE: core::cell::Cell<Option<usize>> =
        const { core::cell::Cell::new(None) };
}

#[cfg(test)]
struct StagingAllocationFailpointReset(Option<usize>);

#[cfg(test)]
impl Drop for StagingAllocationFailpointReset {
    fn drop(&mut self) {
        STAGING_RESERVATIONS_BEFORE_FAILURE.with(|remaining| remaining.set(self.0));
    }
}

/// 只供同线程单元测试确定性覆盖第 N 次切换暂存预留失败。
#[cfg(test)]
pub(crate) fn with_staging_allocation_failure_after<T>(
    successful_reservations: usize,
    run: impl FnOnce() -> T,
) -> T {
    STAGING_RESERVATIONS_BEFORE_FAILURE.with(|remaining| {
        let _reset =
            StagingAllocationFailpointReset(remaining.replace(Some(successful_reservations)));
        run()
    })
}

/// 切换暂存预留：分配失败统一收敛到 [`CutoverError::StagingAllocFailed`]，
/// 由 Prepare 在解除日志武装后返回，旧世界保持可用。
fn try_reserve_staging_exact<T>(values: &mut Vec<T>, capacity: usize) -> Result<(), CutoverError> {
    if capacity == 0 {
        return Ok(());
    }
    #[cfg(test)]
    {
        let fail = STAGING_RESERVATIONS_BEFORE_FAILURE.with(|remaining| match remaining.get() {
            Some(0) => true,
            Some(value) => {
                remaining.set(Some(value - 1));
                false
            }
            None => false,
        });
        if fail {
            return Err(CutoverError::StagingAllocFailed);
        }
    }
    values
        .try_reserve_exact(capacity)
        .map_err(|_| CutoverError::StagingAllocFailed)
}

/// base→target 稳定引用重绑表：按实体种类分列的稠密序数映射。
///
/// 构造自两侧 `SharedIdentityIndex`（切换合同 §2：base 侧来自活动聚合、
/// target 侧随候选；等价证明中共同复核稳定引用映射）。base 序数在 target
/// 无对应稳定引用时为 `None`——使用处即「引用不存在」失败关闭点。
pub(crate) struct CrossRevisionRebinding {
    lane_edges: Vec<Option<LaneEdgeOrdinal>>,
    parking_facilities: Vec<Option<ParkingFacilityOrdinal>>,
    parking_spaces: Vec<Option<ParkingSpaceOrdinal>>,
    vehicle_profiles: Vec<Option<VehicleProfileOrdinal>>,
    participant_classes: Vec<Option<ParticipantClassOrdinal>>,
}

fn map_kind<K>(
    base: &SharedIdentityIndex,
    target: &SharedIdentityIndex,
    kind: EntityKind,
) -> Result<Vec<Option<Ordinal<K>>>, CutoverError>
where
    K: EntityKindMarker + OrdinalKind,
{
    let count = usize::try_from(base.entity_count(kind)).expect("entity count fits usize");
    let mut map = Vec::new();
    try_reserve_staging_exact(&mut map, count)?;
    for raw in 0..base.entity_count(kind) {
        let ordinal = Ordinal::<K>::from_raw(raw);
        let stable = base
            .stable_id(ordinal)
            .expect("dense identity table resolves in-bounds ordinal");
        map.push(target.ordinal(stable));
    }
    Ok(map)
}

impl CrossRevisionRebinding {
    /// 从两侧身份索引构建重绑表；预留失败按切换暂存失败关闭。
    pub(crate) fn build(
        base: &SharedIdentityIndex,
        target: &SharedIdentityIndex,
    ) -> Result<Self, CutoverError> {
        Ok(Self {
            lane_edges: map_kind(base, target, EntityKind::LaneEdge)?,
            parking_facilities: map_kind(base, target, EntityKind::ParkingFacility)?,
            parking_spaces: map_kind(base, target, EntityKind::ParkingSpace)?,
            vehicle_profiles: map_kind(base, target, EntityKind::VehicleProfile)?,
            participant_classes: map_kind(base, target, EntityKind::ParticipantClass)?,
        })
    }

    /// 释放四张重绑表（失败结算时归还内存；结算后事务不可再消费）。
    pub(crate) fn release(&mut self) {
        self.lane_edges = Vec::new();
        self.parking_facilities = Vec::new();
        self.parking_spaces = Vec::new();
        self.vehicle_profiles = Vec::new();
        self.participant_classes = Vec::new();
    }

    /// 车道边序数重绑；`None` = 引用不存在。
    #[must_use]
    pub(crate) fn lane_edge(&self, base: LaneEdgeOrdinal) -> Option<LaneEdgeOrdinal> {
        self.lane_edges.get(base.index()).copied().flatten()
    }

    /// 停车位序数重绑；`None` = 引用不存在。
    #[must_use]
    pub(crate) fn parking_space(&self, base: ParkingSpaceOrdinal) -> Option<ParkingSpaceOrdinal> {
        self.parking_spaces.get(base.index()).copied().flatten()
    }

    /// 停车设施序数重绑；`None` = 引用不存在。
    #[must_use]
    pub(crate) fn parking_facility(
        &self,
        base: ParkingFacilityOrdinal,
    ) -> Option<ParkingFacilityOrdinal> {
        self.parking_facilities.get(base.index()).copied().flatten()
    }

    /// 车辆 profile 序数重绑；`None` = 引用不存在。
    #[must_use]
    pub(crate) fn vehicle_profile(
        &self,
        base: VehicleProfileOrdinal,
    ) -> Option<VehicleProfileOrdinal> {
        self.vehicle_profiles.get(base.index()).copied().flatten()
    }

    /// 参与者类别序数重绑；`None` = 引用不存在。
    #[must_use]
    pub(crate) fn participant_class(
        &self,
        base: ParticipantClassOrdinal,
    ) -> Option<ParticipantClassOrdinal> {
        self.participant_classes
            .get(base.index())
            .copied()
            .flatten()
    }
}

fn try_clone<T: Clone>(source: &[T]) -> Result<Vec<T>, CutoverError> {
    let mut clone = Vec::new();
    clone
        .try_reserve_exact(source.len())
        .map_err(|_| CutoverError::StagingAllocFailed)?;
    clone.extend_from_slice(source);
    Ok(clone)
}

/// 把 `world` 的动态状态结构克隆到 `target_revision` 上并完成直移。
///
/// 候选与旧世界槽位布局一致（当期 `RouteHandle` / `VehicleHandle` 恒等保持，
/// 切换合同 §3 逻辑恒等）；tick/时间/游标保持克隆时点的基线值。逐实体
/// 重绑即重验证：任一引用不存在、路线重编译失败或车辆原样重绑违反
/// target 不变量（进度越界、超速、后缀访问被拒、与其它迁移车辆重叠）都
/// 返回错误并丢弃候选——旧世界从不被本函数触及。
pub(crate) fn migrate_structural_clone(
    world: &TrafficWorld,
    target_revision: Arc<SharedNetworkRevision>,
    target_source: CommittedNetworkSource,
    rebinding: &CrossRevisionRebinding,
) -> Result<TrafficWorld, CutoverError> {
    let target_traffic = target_revision.traffic();

    // 路线：逐槽位重绑边序数并对 target 根重编译（等价重执行 register_route
    // 的编译检查）。
    let mut routes = Vec::new();
    routes
        .try_reserve_exact(world.routes.len())
        .map_err(|_| CutoverError::StagingAllocFailed)?;
    let mut conflict_occurrence_total = 0_u64;
    for slot in &world.routes {
        let Some(compiled) = slot.compiled.as_ref() else {
            routes.push(RouteSlot {
                generation: slot.generation,
                compiled: None,
                live_vehicles: slot.live_vehicles,
            });
            continue;
        };
        let mut target_edges = Vec::new();
        target_edges
            .try_reserve_exact(compiled.edges.len())
            .map_err(|_| CutoverError::StagingAllocFailed)?;
        for edge in &compiled.edges {
            let Some(target_edge) = rebinding.lane_edge(*edge) else {
                return Err(CutoverError::UnmappableLaneEdge {
                    base_edge: edge.raw(),
                });
            };
            target_edges.push(target_edge);
        }
        let migrated = compile_route(
            target_revision.as_ref(),
            target_edges.as_slice(),
            conflict_occurrence_total,
            world.config.route_conflict_occurrence_capacity(),
        )
        .map_err(|error| match error {
            crate::RouteError::AllocationFailed => CutoverError::StagingAllocFailed,
            crate::RouteError::ConflictOccurrenceCapacityExceeded {
                current,
                added,
                capacity,
            } => CutoverError::ConflictOccurrenceCapacityExceeded {
                total: current.saturating_add(added),
                capacity,
            },
            _ => CutoverError::RouteRevalidationFailed,
        })?;
        conflict_occurrence_total = conflict_occurrence_total
            .checked_add(
                u64::try_from(migrated.conflicts.len())
                    .expect("conflict occurrence count fits u64"),
            )
            .expect("route conflict capacity preflight guarantees room");
        routes.push(RouteSlot {
            generation: slot.generation,
            compiled: Some(migrated),
            live_vehicles: slot.live_vehicles,
        });
    }

    // 车辆：profile / 类别重绑，整值运动状态原样保留。停车由下方唯一 aggregate 重建。
    let mut vehicles = Vec::new();
    vehicles
        .try_reserve_exact(world.vehicles.len())
        .map_err(|_| CutoverError::StagingAllocFailed)?;
    for slot in &world.vehicles {
        let Some(state) = slot.state.as_ref() else {
            vehicles.push(VehicleSlot {
                generation: slot.generation,
                state: None,
            });
            continue;
        };
        let mut migrated = *state;
        migrated.profile = rebinding.vehicle_profile(state.profile).ok_or(
            CutoverError::UnmappableVehicleProfile {
                base_profile: state.profile.raw(),
            },
        )?;
        migrated.class = rebinding.participant_class(state.class).ok_or(
            CutoverError::UnmappableParticipantClass {
                base_class: state.class.raw(),
            },
        )?;
        vehicles.push(VehicleSlot {
            generation: slot.generation,
            state: Some(migrated),
        });
    }

    // 停车 aggregate：显式资源按 stable space 重绑；virtual target 按 stable facility
    // 重绑，Reserved selected entry 再用 exact (stable LaneEdge, progress_mm) 解析。
    let target_space_count = usize::try_from(
        target_traffic
            .entity_counts()
            .count(EntityKind::ParkingSpace),
    )
    .expect("target parking space count fits usize");
    let target_facility_count = usize::try_from(
        target_traffic
            .entity_counts()
            .count(EntityKind::ParkingFacility),
    )
    .expect("target parking facility count fits usize");
    let mut parking = ParkingRuntimeState::try_new(target_space_count, target_facility_count)
        .map_err(|()| CutoverError::StagingAllocFailed)?;
    for vehicle in world.live_order.iter().copied() {
        let Some(binding) = world.parking.binding(vehicle) else {
            continue;
        };
        parking
            .try_reserve_binding()
            .map_err(|()| CutoverError::StagingAllocFailed)?;
        let migrated = match binding {
            ParkingBinding::Reserved(reservation) => {
                let (target, selector) = match reservation.target() {
                    ParkingTarget::ExplicitSpace(base_space) => {
                        let target_space = rebinding.parking_space(base_space).ok_or(
                            CutoverError::UnmappableParkingSpace {
                                base_space: base_space.raw(),
                            },
                        )?;
                        (ParkingTarget::ExplicitSpace(target_space), None)
                    }
                    ParkingTarget::VirtualPool(base_facility) => {
                        let target_facility = rebinding.parking_facility(base_facility).ok_or(
                            CutoverError::UnmappableParkingFacility {
                                base_facility: base_facility.raw(),
                            },
                        )?;
                        let base_selector = reservation.virtual_entry_selector().ok_or(
                            CutoverError::ParkingRevalidationFailed {
                                vehicle: vehicle.index(),
                            },
                        )?;
                        let base_view = world
                            .revision
                            .traffic()
                            .relations()
                            .parking_facility(base_facility)
                            .ok_or(CutoverError::ParkingRevalidationFailed {
                                vehicle: vehicle.index(),
                            })?;
                        let base_anchor = base_view
                            .virtual_entries()
                            .get(base_selector.index())
                            .ok_or(CutoverError::ParkingRevalidationFailed {
                                vehicle: vehicle.index(),
                            })?;
                        let target_edge = rebinding.lane_edge(base_anchor.lane_edge()).ok_or(
                            CutoverError::UnmappableLaneEdge {
                                base_edge: base_anchor.lane_edge().raw(),
                            },
                        )?;
                        let target_view = target_traffic
                            .relations()
                            .parking_facility(target_facility)
                            .ok_or(CutoverError::ParkingRevalidationFailed {
                                vehicle: vehicle.index(),
                            })?;
                        let selector_index = target_view
                            .virtual_entries()
                            .iter()
                            .position(|anchor| {
                                anchor.lane_edge() == target_edge
                                    && anchor.progress_mm() == base_anchor.progress_mm()
                            })
                            .ok_or(CutoverError::ParkingRevalidationFailed {
                                vehicle: vehicle.index(),
                            })?;
                        (
                            ParkingTarget::VirtualPool(target_facility),
                            Some(VirtualEntryAnchorSelector::from_raw(
                                u32::try_from(selector_index)
                                    .expect("virtual entry selector fits u32"),
                            )),
                        )
                    }
                };
                ParkingBinding::Reserved(ParkingReservation::new(
                    target,
                    reservation.route(),
                    reservation.entry_route_occurrence(),
                    selector,
                ))
            }
            ParkingBinding::Occupied(base_target) => {
                let target = match base_target {
                    ParkingTarget::ExplicitSpace(base_space) => {
                        ParkingTarget::ExplicitSpace(rebinding.parking_space(base_space).ok_or(
                            CutoverError::UnmappableParkingSpace {
                                base_space: base_space.raw(),
                            },
                        )?)
                    }
                    ParkingTarget::VirtualPool(base_facility) => ParkingTarget::VirtualPool(
                        rebinding.parking_facility(base_facility).ok_or(
                            CutoverError::UnmappableParkingFacility {
                                base_facility: base_facility.raw(),
                            },
                        )?,
                    ),
                };
                ParkingBinding::Occupied(target)
            }
        };
        match migrated {
            ParkingBinding::Reserved(reservation) => {
                match reservation.target() {
                    ParkingTarget::ExplicitSpace(space) => {
                        if parking.explicit_state(space) != Some(ParkingSpaceState::Vacant) {
                            return Err(CutoverError::ParkingRevalidationFailed {
                                vehicle: vehicle.index(),
                            });
                        }
                    }
                    ParkingTarget::VirtualPool(facility) => {
                        let state = parking.virtual_state(facility).ok_or(
                            CutoverError::ParkingRevalidationFailed {
                                vehicle: vehicle.index(),
                            },
                        )?;
                        let capacity = target_traffic
                            .relations()
                            .parking_facility(facility)
                            .ok_or(CutoverError::ParkingRevalidationFailed {
                                vehicle: vehicle.index(),
                            })?
                            .virtual_capacity();
                        let used = state
                            .reserved_count
                            .checked_add(state.occupied_count)
                            .ok_or(CutoverError::ParkingRevalidationFailed {
                                vehicle: vehicle.index(),
                            })?;
                        if used >= capacity {
                            return Err(CutoverError::ParkingRevalidationFailed {
                                vehicle: vehicle.index(),
                            });
                        }
                    }
                }
                parking.insert_reserved(vehicle, reservation);
            }
            ParkingBinding::Occupied(target) => {
                match target {
                    ParkingTarget::ExplicitSpace(space) => {
                        if parking.explicit_state(space) != Some(ParkingSpaceState::Vacant) {
                            return Err(CutoverError::ParkingRevalidationFailed {
                                vehicle: vehicle.index(),
                            });
                        }
                    }
                    ParkingTarget::VirtualPool(facility) => {
                        let state = parking.virtual_state(facility).ok_or(
                            CutoverError::ParkingRevalidationFailed {
                                vehicle: vehicle.index(),
                            },
                        )?;
                        let capacity = target_traffic
                            .relations()
                            .parking_facility(facility)
                            .ok_or(CutoverError::ParkingRevalidationFailed {
                                vehicle: vehicle.index(),
                            })?
                            .virtual_capacity();
                        let used = state
                            .reserved_count
                            .checked_add(state.occupied_count)
                            .ok_or(CutoverError::ParkingRevalidationFailed {
                                vehicle: vehicle.index(),
                            })?;
                        if used >= capacity {
                            return Err(CutoverError::ParkingRevalidationFailed {
                                vehicle: vehicle.index(),
                            });
                        }
                    }
                }
                parking.insert_occupied(vehicle, target);
            }
        }
    }

    let group_count = usize::try_from(
        target_traffic
            .entity_counts()
            .count(EntityKind::SignalGroup),
    )
    .expect("target signal group count fits usize");

    let mut free_routes = try_clone(world.free_routes.as_slice())?;
    let mut free_vehicles = try_clone(world.free_vehicles.as_slice())?;
    let mut live_order = try_clone(world.live_order.as_slice())?;
    let mut active_order = try_clone(world.active_order.as_slice())?;
    // install 同构容量余量：晋升后的世界在配置容量内的生命周期命令不触发
    // 无检分配；窗口重放的 push 同界（上游注册已受容量约束）。
    let route_capacity = usize::try_from(world.config.route_capacity()).unwrap_or(0);
    let vehicle_capacity = usize::try_from(world.config.vehicle_capacity()).unwrap_or(0);
    routes
        .try_reserve_exact(route_capacity.saturating_sub(routes.len()))
        .map_err(|_| CutoverError::StagingAllocFailed)?;
    free_routes
        .try_reserve_exact(route_capacity.saturating_sub(free_routes.len()))
        .map_err(|_| CutoverError::StagingAllocFailed)?;
    vehicles
        .try_reserve_exact(vehicle_capacity.saturating_sub(vehicles.len()))
        .map_err(|_| CutoverError::StagingAllocFailed)?;
    free_vehicles
        .try_reserve_exact(vehicle_capacity.saturating_sub(free_vehicles.len()))
        .map_err(|_| CutoverError::StagingAllocFailed)?;
    live_order
        .try_reserve_exact(vehicle_capacity.saturating_sub(live_order.len()))
        .map_err(|_| CutoverError::StagingAllocFailed)?;
    active_order
        .try_reserve_exact(vehicle_capacity.saturating_sub(active_order.len()))
        .map_err(|_| CutoverError::StagingAllocFailed)?;
    let mut next_states = Vec::new();
    next_states
        .try_reserve_exact(world.next_states.capacity())
        .map_err(|_| CutoverError::StagingAllocFailed)?;
    let mut signal_aspects = Vec::new();
    try_reserve_staging_exact(&mut signal_aspects, group_count)?;
    signal_aspects.resize(group_count, SignalAspect::Red);

    // occurrence 总数重算（迁移不增减边数；防御性闭合后文校验容量）。
    let mut occurrence_total: u64 = 0;
    for slot in &routes {
        if let Some(compiled) = slot.compiled.as_ref() {
            occurrence_total +=
                u64::try_from(compiled.edges.len()).expect("route edge count fits u64");
        }
    }

    let mut candidate = TrafficWorld {
        revision: target_revision,
        source: target_source,
        world_id: world.world_id,
        world_generation: world.world_generation,
        config: world.config,
        tick_index: world.tick_index,
        time_ms: world.time_ms,
        command_cursor: world.command_cursor,
        event_cursor: world.event_cursor,
        observation_state_sequence: world.observation_state_sequence,
        signal_aspects: signal_aspects.into_boxed_slice(),
        routes,
        free_routes,
        live_route_count: world.live_route_count,
        live_route_edge_occurrence_count: occurrence_total,
        live_route_conflict_occurrence_count: conflict_occurrence_total,
        vehicles,
        free_vehicles,
        live_order,
        active_order,
        parking,
        next_states,
        occupancy: crate::occupancy::OccupancyIndex::with_capacity(0, 0),
        migration_journal: None,
        migration_epoch: 0,
    };
    if candidate.live_route_edge_occurrence_count > world.config.route_edge_occurrence_capacity() {
        return Err(CutoverError::EdgeOccurrenceCapacityExceeded {
            total: candidate.live_route_edge_occurrence_count,
            capacity: world.config.route_edge_occurrence_capacity(),
        });
    }
    candidate.refresh_signals();
    revalidate_migrated_vehicles(&candidate)?;
    Ok(candidate)
}

/// 重绑即重验证：对候选内每个 live 车辆按其生命周期状态复核 target
/// 不变量（切换合同 §3 原则 2，等价重执行 spawn 检查）。
pub(crate) fn revalidate_migrated_vehicles(candidate: &TrafficWorld) -> Result<(), CutoverError> {
    for handle in candidate.live_order.iter().copied() {
        revalidate_vehicle_on(candidate, handle)?;
    }
    Ok(())
}

/// 单个车辆在候选上的重验证（切片 C-3 的增量重放复用同一入口）。
///
/// 复核面：路线解析、序列下标与进度对 target 边长、Active 的限速与后缀
/// 访问、Active 与其它 Active 的重叠（按 live 序线性扫描）。
pub(crate) fn revalidate_vehicle_on(
    candidate: &TrafficWorld,
    handle: VehicleHandle,
) -> Result<(), CutoverError> {
    let traffic = candidate.revision.traffic();
    let lengths = traffic.lane_lengths_millimetres();
    let speed_limits = traffic.lane_speed_limits_millimetres_per_second();
    let state = candidate
        .vehicle_state(handle)
        .ok_or(CutoverError::VehicleRevalidationFailed {
            vehicle: handle.index(),
        })?;
    if !candidate.parking_state_valid(handle) {
        return Err(CutoverError::ParkingRevalidationFailed {
            vehicle: handle.index(),
        });
    }
    // 派生不变量（重执行 spawn 检查的派生面）：target profile 派生的
    // class/length 必须与车辆存量一致，否则不可映射——直移保留存量会
    // 与 save/restore 的派生态分歧（class 侧恢复被拒、length 侧静默漂移）。
    let profile = traffic.relations().vehicle_profile(state.profile).ok_or(
        CutoverError::ProfileDerivationMismatch {
            vehicle: handle.index(),
        },
    )?;
    if profile.class() != state.class || profile.length_mm() != state.length_mm {
        return Err(CutoverError::ProfileDerivationMismatch {
            vehicle: handle.index(),
        });
    }
    let edges =
        candidate
            .route_edges(state.route)
            .ok_or(CutoverError::VehicleRevalidationFailed {
                vehicle: handle.index(),
            })?;
    let cursor = usize::try_from(state.route_edge_index).map_err(|_| {
        CutoverError::VehicleRevalidationFailed {
            vehicle: handle.index(),
        }
    })?;
    let Some(edge) = edges.get(cursor).copied() else {
        return Err(CutoverError::VehicleRevalidationFailed {
            vehicle: handle.index(),
        });
    };
    if state.progress_mm > lengths[edge.index()] {
        return Err(CutoverError::VehicleRevalidationFailed {
            vehicle: handle.index(),
        });
    }
    // 全状态闭合（恢复兼容）：快照恢复对每个车辆（含 Parked/Completed）
    // 都经 spawn_vehicle 重建，速度上限与后缀准入对全部状态生效——迁移后
    // 存量必须同样满足，否则晋升的是恢复不兼容态。
    if state.speed_mm_s > speed_limits[edge.index()] {
        return Err(CutoverError::VehicleRevalidationFailed {
            vehicle: handle.index(),
        });
    }
    let compiled = candidate
        .compiled_route(state.route)
        .expect("live vehicle route stays compiled");
    if route_access_denied(
        traffic,
        state.class,
        edges,
        cursor,
        compiled
            .maneuvers
            .iter()
            .map(|occurrence| (occurrence.path, occurrence.exit_route_edge_index)),
    ) {
        return Err(CutoverError::VehicleRevalidationFailed {
            vehicle: handle.index(),
        });
    }
    // Completed 恰在末边末端：与恢复侧 InvalidCompletedState 同判据
    //（目标加长末边时，旧端点不再在末端，属不可映射）。
    if state.status == VehicleStatus::Completed
        && (cursor + 1 != edges.len() || state.progress_mm != lengths[edge.index()])
    {
        return Err(CutoverError::VehicleRevalidationFailed {
            vehicle: handle.index(),
        });
    }
    if state.status == VehicleStatus::Active {
        for other in candidate.live_order.iter().copied() {
            if other == handle {
                continue;
            }
            let Some(leader) = candidate.vehicle_state(other) else {
                continue;
            };
            if leader.status != VehicleStatus::Active {
                continue;
            }
            let Some(leader_edges) = candidate.route_edges(leader.route) else {
                continue;
            };
            let Ok(leader_cursor) = usize::try_from(leader.route_edge_index) else {
                continue;
            };
            if bodies_overlap(
                lengths,
                edges,
                cursor,
                state.progress_mm,
                state.length_mm,
                leader_edges,
                leader_cursor,
                leader.progress_mm,
                leader.length_mm,
            ) {
                return Err(CutoverError::VehicleRevalidationFailed {
                    vehicle: handle.index(),
                });
            }
        }
        match candidate.check_active_conflict_capability(
            state.route,
            cursor,
            state.progress_mm,
            state.carry_um,
            state.length_mm,
        ) {
            Ok(()) => {}
            Err(crate::tables::ConflictCapabilityError::InvalidCursor) => {
                return Err(CutoverError::VehicleRevalidationFailed {
                    vehicle: handle.index(),
                });
            }
            Err(crate::tables::ConflictCapabilityError::RuntimeUnavailable(error)) => {
                return Err(CutoverError::ConflictRuntimeUnavailable(error));
            }
        }
    }
    Ok(())
}

/// 统一的已提交逻辑状态比较器（切片 C 测试共用；含信号灯色组轴）。
#[cfg(test)]
pub(crate) fn assert_committed_logical_state_equal(left: &TrafficWorld, right: &TrafficWorld) {
    assert_eq!(left.tick_index(), right.tick_index());
    assert_eq!(left.time_ms(), right.time_ms());
    assert_eq!(left.live_vehicles(), right.live_vehicles());
    for handle in right.live_vehicles() {
        assert_eq!(
            left.vehicle_state(*handle).expect("vehicle"),
            right.vehicle_state(*handle).expect("vehicle"),
        );
    }
    assert_eq!(
        left.committed_pose_sources().as_slice(),
        right.committed_pose_sources().as_slice(),
    );
    assert_eq!(
        left.committed_signal_groups().as_slice(),
        right.committed_signal_groups().as_slice(),
    );
    let spaces = usize::try_from(
        right
            .traffic()
            .entity_counts()
            .count(laneflow_static_contract::EntityKind::ParkingSpace),
    )
    .expect("space count");
    for raw in 0..spaces {
        let space = laneflow_static_contract::ParkingSpaceOrdinal::from_raw(
            u32::try_from(raw).expect("fits u32"),
        );
        assert_eq!(
            left.committed_parking_occupant(space),
            right.committed_parking_occupant(space),
        );
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use laneflow_compiler::{
        CompilationUnitBuilder, CompileLimits, Compiler, IidmVehicleProfileInput, LaneEdgeInput,
        LaneEdgeReference, ParkingFacilityInput, ParkingLaneAnchorInput, ParkingSpaceGeometryInput,
        ParkingSpaceInput, ParticipantClassInput, ParticipantClassReference, PortableDiffBase,
        PortableEmissionProvenance, SourceModuleHeader, SourceModuleHeaderInput,
        SyntheticModuleBuilder, VehicleProfileInput, emit_portable_candidate,
    };
    use laneflow_format::{
        FormatLimits, check_canonical_network_input, check_post_emission_bundle,
    };
    use laneflow_static_contract::{
        ExactByteLength, SEMANTIC_DIFF_FORMAT_VERSION, Sha256Digest, StableId128,
        VehicleProfileOrdinal,
    };
    use laneflow_static_network::{
        CanonicalNetworkOrigin, SharedNetworkBuildLimits, SharedNetworkBuildOptions,
        SpatialBuildOption, build_shared_network_revision,
    };
    use sha2::Digest as _;

    use super::*;
    use crate::{
        CUTOVER_DESCRIPTOR_FORMAT_VERSION, LfcaOriginBinding, MigrationPolicyKind,
        NetworkRevisionCutoverDescriptor, ParkedVehicleSpawnInput, ParkingTarget, PoseSource,
        ReserveParkingTarget, RouteRegisterInput, SemanticDiffOriginBinding, TickInput,
        VehicleSpawnInput, VirtualEntryAnchorSelector, WorldBinding, WorldConfig, WorldGeneration,
    };

    const BASE: &[u8] =
        include_bytes!("../../laneflow-compiler/tests/fixtures/portable/lfsd-migration/base.lfca");
    const TARGET: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfsd-migration/target.lfca"
    );
    const LFSD_BYTES: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfsd-migration/expected.lfsd"
    );
    const ORACLE_BASE: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfsd-migration/oracle-base.lfca"
    );
    const ORACLE_TARGET: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfsd-migration/oracle-target.lfca"
    );
    const PROFILE_BASE: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfsd-migration/profile-base.lfca"
    );
    const PROFILE_TARGET: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfsd-migration/profile-target.lfca"
    );

    fn revision(bytes: &[u8]) -> Arc<SharedNetworkRevision> {
        let input = check_canonical_network_input(bytes, FormatLimits::HARD)
            .expect("checked canonical network input");
        build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::Omit,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .expect("shared network revision")
    }

    #[derive(Clone, Copy)]
    pub(crate) enum ParkingRevisionShape {
        Facility {
            capacity: u32,
            entry_progress_m: f64,
        },
        Missing,
        WrongKind,
    }

    pub(crate) fn compiled_parking_revision(
        shape: ParkingRevisionShape,
    ) -> Arc<SharedNetworkRevision> {
        let limits = CompileLimits::p100_initial_v1();
        let digest_byte = match shape {
            ParkingRevisionShape::Facility { capacity, .. } => {
                u8::try_from(capacity.min(250)).expect("clamped")
            }
            ParkingRevisionShape::Missing => 251,
            ParkingRevisionShape::WrongKind => 252,
        };
        let header = SourceModuleHeader::new(
            SourceModuleHeaderInput {
                authoring_namespace_id: "city/parking-cutover",
                source_document_key: "parking-cutover.document",
                generator_build_id: "git:0123456789abcdef",
                parameters_and_inputs_digest: [digest_byte; 32],
                frontend_options_digest: [0x54; 32],
                random_seed: Some(541),
                provenance: "repository:laneflow",
            },
            &limits,
        )
        .expect("parking source header");
        let mut module = SyntheticModuleBuilder::new(header, &limits).expect("parking module");
        module
            .add_participant_class(ParticipantClassInput {
                participant_class_key: "road-user",
                extends: None,
            })
            .expect("class")
            .add_vehicle_profile(VehicleProfileInput {
                vehicle_profile_key: "car",
                participant_class: ParticipantClassReference::local("road-user"),
                iidm: IidmVehicleProfileInput {
                    length_meters: 4.5,
                    desired_speed_meters_per_second: 13.75,
                    min_gap_meters: 2.0,
                    time_headway_seconds: 1.4,
                    max_acceleration_meters_per_second_squared: 1.8,
                    comfortable_deceleration_meters_per_second_squared: 2.0,
                    emergency_deceleration_meters_per_second_squared: 4.5,
                },
            })
            .expect("profile")
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge",
                length_meters: 100.0,
                speed_limit_meters_per_second: 15.0,
                successors: &[],
            })
            .expect("edge");
        match shape {
            ParkingRevisionShape::Facility {
                capacity,
                entry_progress_m,
            } => {
                let entries = [ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local("edge"),
                    progress_meters: entry_progress_m,
                }];
                let exits = [ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local("edge"),
                    progress_meters: 80.0,
                }];
                module
                    .add_parking_facility(ParkingFacilityInput {
                        parking_facility_key: "facility",
                        virtual_capacity: capacity,
                        virtual_entries: &entries,
                        virtual_exits: &exits,
                    })
                    .expect("facility");
            }
            ParkingRevisionShape::Missing => {}
            ParkingRevisionShape::WrongKind => {
                module
                    .add_parking_space(ParkingSpaceInput {
                        parking_space_key: "facility",
                        parking_facility: None,
                        entry: ParkingLaneAnchorInput {
                            lane_edge: LaneEdgeReference::local("edge"),
                            progress_meters: 20.0,
                        },
                        exit: ParkingLaneAnchorInput {
                            lane_edge: LaneEdgeReference::local("edge"),
                            progress_meters: 80.0,
                        },
                        geometry: ParkingSpaceGeometryInput {
                            lateral_offset_meters: -3.0,
                            heading_offset_radians: 0.0,
                            length_meters: 5.5,
                            width_meters: 2.6,
                        },
                    })
                    .expect("wrong-kind space");
            }
        }

        let mut unit = CompilationUnitBuilder::new(limits);
        unit.add_synthetic_module(module.finish().expect("finished parking module"))
            .expect("parking compilation module");
        let output = Compiler::new()
            .compile(unit.build().expect("parking compilation unit"))
            .expect("compiled parking revision");
        let provenance =
            PortableEmissionProvenance::try_new("parking-cutover-v1").expect("parking provenance");
        let candidate = emit_portable_candidate(
            &output,
            &provenance,
            FormatLimits::HARD,
            PortableDiffBase::Genesis,
        )
        .expect("parking candidate");
        let checked = check_post_emission_bundle(
            candidate.canonical_artifact().bytes(),
            candidate.source_map().bytes(),
            candidate.semantic_diff().bytes(),
            candidate.expected_semantic_diff_base(),
            FormatLimits::HARD,
        )
        .expect("checked parking bundle");
        build_shared_network_revision(
            checked.canonical_network_input(),
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::Omit,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .expect("parking shared revision")
    }

    fn source_for(origin: CanonicalNetworkOrigin, key: &str) -> CommittedNetworkSource {
        CommittedNetworkSource::Published {
            reference: crate::PublishedLfcaReference::new(
                key,
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("non-empty key"),
        }
    }

    fn lfsd_binding(bytes: &[u8]) -> SemanticDiffOriginBinding {
        let digest: [u8; 32] = sha2::Sha256::digest(bytes).into();
        SemanticDiffOriginBinding::new(
            SEMANTIC_DIFF_FORMAT_VERSION,
            Sha256Digest::from_bytes(digest),
            ExactByteLength::new(u64::try_from(bytes.len()).expect("fixture length fits u64")),
        )
    }

    fn cross_revision_descriptor(
        base_origin: CanonicalNetworkOrigin,
        target_origin: CanonicalNetworkOrigin,
        binding: SemanticDiffOriginBinding,
    ) -> NetworkRevisionCutoverDescriptor {
        NetworkRevisionCutoverDescriptor::new(
            LfcaOriginBinding::from_canonical_origin(base_origin),
            LfcaOriginBinding::from_canonical_origin(target_origin),
            Some(binding),
            MigrationPolicyKind::CrossRevisionDirect,
            WorldBinding::new(0, WorldGeneration::INITIAL, 0, 0),
        )
    }

    const FULL_SPATIAL_LFCA: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfca"
    );

    #[test]
    fn signal_aspect_staging_reservation_failure_fails_closed() {
        // full-spatial 带信号组（group_count > 0）：重绑表已在外层构建，
        // 迁移内的首次切换暂存预留即候选信号灯色切片。
        let world = installed_world(FULL_SPATIAL_LFCA, "fixture://signal-stage");
        let target = revision(FULL_SPATIAL_LFCA);
        let target_origin = *target.canonical_origin();
        let rebinding =
            CrossRevisionRebinding::build(world.revision.identity(), target.identity()).unwrap();
        let result = with_staging_allocation_failure_after(0, || {
            migrate_structural_clone(
                &world,
                Arc::clone(&target),
                source_for(target_origin, "fixture://signal-stage-target"),
                &rebinding,
            )
        });
        assert_eq!(
            match result {
                Err(error) => error,
                Ok(_) => panic!("signal staging failure must fail closed"),
            },
            CutoverError::StagingAllocFailed
        );
    }

    fn installed_world(bytes: &[u8], key: &str) -> TrafficWorld {
        let revision = revision(bytes);
        let origin = *revision.canonical_origin();
        TrafficWorld::install(
            revision,
            WorldConfig::new(8, 4, 1_024, 1_024, 1, 100),
            source_for(origin, key),
            0,
        )
        .expect("install")
    }

    fn entry_exit(world: &TrafficWorld) -> (LaneEdgeOrdinal, LaneEdgeOrdinal) {
        let traffic = world.traffic();
        for raw in 0..traffic.lane_edge_count() {
            let edge = LaneEdgeOrdinal::from_raw(raw);
            if let Some(successor) = traffic
                .successors(edge)
                .and_then(|items| items.first().copied())
            {
                return (edge, successor);
            }
        }
        panic!("fixture exposes a connected edge pair");
    }

    fn unmappable_edge(
        world: &TrafficWorld,
        rebinding: &CrossRevisionRebinding,
    ) -> LaneEdgeOrdinal {
        for raw in 0..world.traffic().lane_edge_count() {
            let edge = LaneEdgeOrdinal::from_raw(raw);
            if rebinding.lane_edge(edge).is_none() {
                return edge;
            }
        }
        panic!("fixture exposes an unmappable edge");
    }

    fn parking_spaces(
        world: &TrafficWorld,
        rebinding: &CrossRevisionRebinding,
    ) -> (ParkingSpaceOrdinal, ParkingSpaceOrdinal) {
        let count = world
            .traffic()
            .entity_counts()
            .count(EntityKind::ParkingSpace);
        let mut retained = None;
        let mut removed = None;
        for raw in 0..count {
            let space = ParkingSpaceOrdinal::from_raw(raw);
            if rebinding.parking_space(space).is_none() {
                removed = Some(space);
            } else {
                retained = Some(space);
            }
        }
        (
            retained.expect("retained space"),
            removed.expect("removed space"),
        )
    }

    fn stable_edge(world: &TrafficWorld, edge: LaneEdgeOrdinal) -> StableId128 {
        *world
            .revision
            .identity()
            .stable_id(edge)
            .expect("edge ordinal resolves")
            .as_untyped()
    }

    fn stable_space(world: &TrafficWorld, space: ParkingSpaceOrdinal) -> StableId128 {
        *world
            .revision
            .identity()
            .stable_id(space)
            .expect("space ordinal resolves")
            .as_untyped()
    }

    fn stable_profile(world: &TrafficWorld, profile: VehicleProfileOrdinal) -> StableId128 {
        *world
            .revision
            .identity()
            .stable_id(profile)
            .expect("profile ordinal resolves")
            .as_untyped()
    }

    fn stable_pose_batch(world: &TrafficWorld) -> Vec<(VehicleHandle, (StableId128, u32))> {
        world
            .committed_pose_sources()
            .as_slice()
            .iter()
            .map(|(handle, source)| match source {
                PoseSource::Lane { edge, progress_mm } => {
                    (*handle, (stable_edge(world, *edge), *progress_mm))
                }
                PoseSource::Parking { space } => (*handle, (stable_space(world, *space), 0)),
            })
            .collect()
    }

    fn spawn_on(
        world: &mut TrafficWorld,
        route: crate::RouteHandle,
        progress: u32,
        speed: u32,
    ) -> VehicleHandle {
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                progress,
                speed,
            ))
            .expect("spawn")
    }

    fn expect_migration_error(
        world: &TrafficWorld,
        target_revision: Arc<SharedNetworkRevision>,
        rebinding: &CrossRevisionRebinding,
    ) -> CutoverError {
        let target_origin = *target_revision.canonical_origin();
        match migrate_structural_clone(
            world,
            target_revision,
            source_for(target_origin, "fixture://expect-error"),
            rebinding,
        ) {
            Err(error) => error,
            Ok(_) => panic!("expected migration to fail closed"),
        }
    }

    pub(crate) fn virtual_parking_cutover_world() -> (TrafficWorld, VehicleHandle, VehicleHandle) {
        let revision = compiled_parking_revision(ParkingRevisionShape::Facility {
            capacity: 3,
            entry_progress_m: 20.0,
        });
        let origin = *revision.canonical_origin();
        let mut world = TrafficWorld::install(
            revision,
            WorldConfig::new(8, 4, 1_024, 1_024, 1, 100),
            source_for(origin, "fixture://parking-cutover-base"),
            0,
        )
        .expect("parking cutover install");
        let route = world
            .register_route(RouteRegisterInput::new(vec![LaneEdgeOrdinal::from_raw(0)]))
            .expect("parking route");
        let reserved = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                0,
                0,
            ))
            .expect("reserved vehicle");
        world
            .reserve_parking(
                reserved,
                ReserveParkingTarget::VirtualPool {
                    facility: laneflow_static_contract::ParkingFacilityOrdinal::from_raw(0),
                    entry_anchor: VirtualEntryAnchorSelector::from_raw(0),
                    entry_route_occurrence: 0,
                },
            )
            .expect("virtual reservation");
        let occupied = world
            .spawn_parked_vehicle(
                ParkedVehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 0),
                ParkingTarget::VirtualPool(
                    laneflow_static_contract::ParkingFacilityOrdinal::from_raw(0),
                ),
            )
            .expect("virtual occupancy")
            .vehicle;
        (world, reserved, occupied)
    }

    #[test]
    fn virtual_parking_cutover_closes_capacity_identity_and_anchor_changes() {
        let (world, reserved, occupied) = virtual_parking_cutover_world();
        let facility = laneflow_static_contract::ParkingFacilityOrdinal::from_raw(0);
        let before = world.capture_snapshot().expect("base snapshot");

        for capacity in [4, 2] {
            let target = compiled_parking_revision(ParkingRevisionShape::Facility {
                capacity,
                entry_progress_m: 20.0,
            });
            let rebinding =
                CrossRevisionRebinding::build(world.revision.identity(), target.identity())
                    .expect("parking rebinding");
            let target_origin = *target.canonical_origin();
            let candidate = migrate_structural_clone(
                &world,
                target,
                source_for(target_origin, "fixture://parking-cutover-safe"),
                &rebinding,
            )
            .expect("capacity increase/exact decrease remains safe");
            let counts = candidate
                .parking_facility_counts(facility)
                .expect("candidate counts")
                .virtual_pool;
            assert_eq!(
                (counts.capacity, counts.reserved, counts.occupied),
                (u64::from(capacity), 1, 1)
            );
            assert!(matches!(
                candidate.parking_binding(reserved),
                Some(ParkingBinding::Reserved(reservation))
                    if reservation.virtual_entry_selector()
                        == Some(VirtualEntryAnchorSelector::from_raw(0))
            ));
            assert_eq!(
                candidate.parking_binding(occupied),
                Some(ParkingBinding::Occupied(ParkingTarget::VirtualPool(
                    facility
                )))
            );
            assert_eq!(candidate.active_order, [reserved]);
        }

        let unsafe_capacity = compiled_parking_revision(ParkingRevisionShape::Facility {
            capacity: 1,
            entry_progress_m: 20.0,
        });
        let unsafe_rebinding =
            CrossRevisionRebinding::build(world.revision.identity(), unsafe_capacity.identity())
                .expect("unsafe capacity rebinding");
        assert!(matches!(
            expect_migration_error(&world, unsafe_capacity, &unsafe_rebinding),
            CutoverError::ParkingRevalidationFailed { .. }
        ));
        assert_eq!(
            world.capture_snapshot().expect("after unsafe decrease"),
            before
        );

        let moved_anchor = compiled_parking_revision(ParkingRevisionShape::Facility {
            capacity: 3,
            entry_progress_m: 21.0,
        });
        let moved_rebinding =
            CrossRevisionRebinding::build(world.revision.identity(), moved_anchor.identity())
                .expect("moved anchor rebinding");
        assert_eq!(
            expect_migration_error(&world, moved_anchor, &moved_rebinding),
            CutoverError::ParkingRevalidationFailed {
                vehicle: reserved.index()
            }
        );
        assert_eq!(
            world.capture_snapshot().expect("after moved anchor"),
            before
        );

        for shape in [
            ParkingRevisionShape::Missing,
            ParkingRevisionShape::WrongKind,
        ] {
            let target = compiled_parking_revision(shape);
            let rebinding =
                CrossRevisionRebinding::build(world.revision.identity(), target.identity())
                    .expect("missing target rebinding");
            assert_eq!(
                expect_migration_error(&world, target, &rebinding),
                CutoverError::UnmappableParkingFacility { base_facility: 0 }
            );
            assert_eq!(world.capture_snapshot().expect("zero publish"), before);
        }
    }

    #[test]
    fn verify_semantic_diff_accepts_authentic_pairs() {
        let base = revision(BASE);
        let target = revision(TARGET);
        let descriptor = cross_revision_descriptor(
            *base.canonical_origin(),
            *target.canonical_origin(),
            lfsd_binding(LFSD_BYTES),
        );
        assert_eq!(
            descriptor.format_version(),
            CUTOVER_DESCRIPTOR_FORMAT_VERSION
        );
        assert_eq!(
            descriptor.verify_semantic_diff(
                LFSD_BYTES,
                *base.canonical_origin(),
                *target.canonical_origin()
            ),
            Ok(())
        );
    }

    #[test]
    fn verify_semantic_diff_rejects_inconsistent_bytes() {
        let base = revision(BASE);
        let target = revision(TARGET);
        let base_origin = *base.canonical_origin();
        let target_origin = *target.canonical_origin();

        // 摘要不符：翻转一个字节但绑定保持原声明。
        let mut tampered = LFSD_BYTES.to_vec();
        let last = tampered.len() - 1;
        tampered[last] ^= 0x01;
        let descriptor =
            cross_revision_descriptor(base_origin, target_origin, lfsd_binding(LFSD_BYTES));
        assert_eq!(
            descriptor.verify_semantic_diff(&tampered, base_origin, target_origin),
            Err(crate::CutoverDescriptorError::SemanticDiffDigestMismatch)
        );

        // 长度不符。
        let wrong_length = SemanticDiffOriginBinding::new(
            SEMANTIC_DIFF_FORMAT_VERSION,
            lfsd_binding(LFSD_BYTES).semantic_diff_digest(),
            ExactByteLength::new(u64::try_from(LFSD_BYTES.len() + 1).expect("fits")),
        );
        let descriptor = cross_revision_descriptor(base_origin, target_origin, wrong_length);
        assert_eq!(
            descriptor.verify_semantic_diff(LFSD_BYTES, base_origin, target_origin),
            Err(
                crate::CutoverDescriptorError::SemanticDiffByteLengthMismatch {
                    declared: u64::try_from(LFSD_BYTES.len() + 1).expect("fits"),
                    actual: u64::try_from(LFSD_BYTES.len()).expect("fits"),
                }
            )
        );

        // 两侧 origin 对调：base 侧绑定比对失败。
        let descriptor =
            cross_revision_descriptor(base_origin, target_origin, lfsd_binding(LFSD_BYTES));
        assert_eq!(
            descriptor.verify_semantic_diff(LFSD_BYTES, target_origin, base_origin),
            Err(crate::CutoverDescriptorError::SemanticDiffBaseBindingMismatch)
        );
    }

    #[test]
    fn rebinding_maps_retained_and_reports_removed_references() {
        let base = revision(BASE);
        let target = revision(TARGET);
        let rebinding = CrossRevisionRebinding::build(base.identity(), target.identity()).unwrap();
        let world = installed_world(BASE, "fixture://rebinding");
        let target_world = installed_world(TARGET, "fixture://rebinding-target");
        let doomed = unmappable_edge(&world, &rebinding);
        assert!(rebinding.lane_edge(doomed).is_none());
        let (entry, exit) = entry_exit(&world);
        for edge in [entry, exit] {
            let mapped = rebinding.lane_edge(edge).expect("retained edge maps");
            assert_eq!(
                stable_edge(&world, edge),
                stable_edge(&target_world, mapped)
            );
        }
        let (space_main, space_doomed) = parking_spaces(&world, &rebinding);
        assert!(rebinding.parking_space(space_doomed).is_none());
        assert!(rebinding.parking_space(space_main).is_some());

        // oracle 对：交通引用全部恒等映射（含 profile/类别）。
        let oracle_base = revision(ORACLE_BASE);
        let oracle_target = revision(ORACLE_TARGET);
        let oracle_rebinding =
            CrossRevisionRebinding::build(oracle_base.identity(), oracle_target.identity())
                .unwrap();
        let oracle_world = installed_world(ORACLE_BASE, "fixture://oracle-rebinding");
        let (entry, exit) = entry_exit(&oracle_world);
        for edge in [entry, exit] {
            assert_eq!(oracle_rebinding.lane_edge(edge), Some(edge));
        }
        let count = oracle_world
            .traffic()
            .entity_counts()
            .count(EntityKind::ParkingSpace);
        for raw in 0..count {
            let space = ParkingSpaceOrdinal::from_raw(raw);
            assert_eq!(oracle_rebinding.parking_space(space), Some(space));
        }
        let profile_count = oracle_world
            .traffic()
            .entity_counts()
            .count(EntityKind::VehicleProfile);
        for raw in 0..profile_count {
            let profile = VehicleProfileOrdinal::from_raw(raw);
            assert_eq!(oracle_rebinding.vehicle_profile(profile), Some(profile));
        }
    }

    #[test]
    fn candidate_tables_carry_install_parity_capacity_headroom() {
        let mut world = installed_world(BASE, "fixture://headroom");
        let (entry, _exit) = entry_exit(&world);
        let route = world
            .register_route(RouteRegisterInput::new(vec![entry]))
            .expect("route");
        spawn_on(&mut world, route, 10_000, 5_000);
        let target_revision = revision(TARGET);
        let rebinding =
            CrossRevisionRebinding::build(world.revision.identity(), target_revision.identity())
                .unwrap();
        let target_origin = *target_revision.canonical_origin();
        let candidate = migrate_structural_clone(
            &world,
            Arc::clone(&target_revision),
            source_for(target_origin, "fixture://headroom-target"),
            &rebinding,
        )
        .expect("migrate");
        // 与 install 同构：晋升后的世界在配置容量内的命令不触发无检分配。
        let route_capacity = usize::try_from(candidate.config.route_capacity()).unwrap();
        let vehicle_capacity = usize::try_from(candidate.config.vehicle_capacity()).unwrap();
        assert!(candidate.routes.capacity() >= route_capacity);
        assert!(candidate.free_routes.capacity() >= route_capacity);
        assert!(candidate.vehicles.capacity() >= vehicle_capacity);
        assert!(candidate.free_vehicles.capacity() >= vehicle_capacity);
        assert!(candidate.live_order.capacity() >= vehicle_capacity);
        assert!(candidate.active_order.capacity() >= vehicle_capacity);
    }

    #[test]
    fn completed_vehicle_off_route_end_fails_revalidation() {
        // Completed 恰在末边末端（恢复侧 InvalidCompletedState 同判据）：
        // 任意非末端形态——含目标加长末边后旧端点成为中段——都不可映射。
        let mut world = installed_world(BASE, "fixture://completed-end");
        let (entry, _exit) = entry_exit(&world);
        let route = world
            .register_route(RouteRegisterInput::new(vec![entry]))
            .expect("route");
        let handle = spawn_on(&mut world, route, 10_000, 5_000);
        let target_revision = revision(TARGET);
        let rebinding =
            CrossRevisionRebinding::build(world.revision.identity(), target_revision.identity())
                .unwrap();
        let target_origin = *target_revision.canonical_origin();
        let mut candidate = migrate_structural_clone(
            &world,
            Arc::clone(&target_revision),
            source_for(target_origin, "fixture://completed-end-target"),
            &rebinding,
        )
        .expect("active vehicle migration succeeds");
        let index = usize::try_from(handle.index()).expect("index");
        candidate.vehicles[index]
            .state
            .as_mut()
            .expect("vehicle")
            .status = VehicleStatus::Completed;
        assert_eq!(
            revalidate_vehicle_on(&candidate, handle),
            Err(CutoverError::VehicleRevalidationFailed {
                vehicle: handle.index()
            })
        );
        // 恰在末端（目标 entry 60 m = 60_000 mm）即恢复兼容。
        candidate.vehicles[index]
            .state
            .as_mut()
            .expect("vehicle")
            .progress_mm = 60_000;
        assert_eq!(revalidate_vehicle_on(&candidate, handle), Ok(()));
    }

    #[test]
    fn migrate_happy_world_preserves_handles_and_logical_state() {
        let mut world = installed_world(BASE, "fixture://migration-base");
        let (entry, _exit) = entry_exit(&world);
        // 快乐路径使用单边路线：target 对 exit 追加了 passenger-car 拒绝，
        // 含 exit 的后缀在重验证时必然被拒（那是访问翻转用例的舞台）。
        let route = world
            .register_route(RouteRegisterInput::new(vec![entry]))
            .expect("route");
        let leader = spawn_on(&mut world, route, 20_000, 5_000);
        // Completed 必须恰在末端：base entry 100 m，target 缩至 60 m——
        // 停在目标末端（60_000 mm）对两侧判据同时成立。
        let completed = spawn_on(&mut world, route, 60_000, 5_000);
        let target_revision = revision(TARGET);
        let rebinding =
            CrossRevisionRebinding::build(world.revision.identity(), target_revision.identity())
                .unwrap();
        let (space_main, _) = parking_spaces(&world, &rebinding);
        let parked = world
            .spawn_parked_vehicle(
                ParkedVehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 2_000),
                ParkingTarget::ExplicitSpace(space_main),
            )
            .expect("parking")
            .vehicle;
        let completed_index = usize::try_from(completed.index()).expect("index");
        world.vehicles[completed_index]
            .state
            .as_mut()
            .expect("completed")
            .status = VehicleStatus::Completed;
        world.rebuild_active_order();
        assert!(!world.active_order.contains(&completed));
        for _ in 0..2 {
            world.step(TickInput::new(100)).expect("step");
        }

        let target_origin = *target_revision.canonical_origin();
        let candidate = migrate_structural_clone(
            &world,
            Arc::clone(&target_revision),
            source_for(target_origin, "fixture://migration-target"),
            &rebinding,
        )
        .expect("migration succeeds");

        // 根与来源换绑；句柄恒等；整值状态逐字段保持。
        assert!(Arc::ptr_eq(&candidate.revision, &target_revision));
        for handle in [leader, completed, parked] {
            let before = world.vehicle_state(handle).expect("vehicle");
            let after = candidate.vehicle_state(handle).expect("vehicle");
            assert_eq!(before.handle, after.handle);
            assert_eq!(before.route, after.route);
            assert_eq!(before.route_edge_index, after.route_edge_index);
            assert_eq!(before.progress_mm, after.progress_mm);
            assert_eq!(before.carry_um, after.carry_um);
            assert_eq!(before.speed_mm_s, after.speed_mm_s);
            assert_eq!(before.length_mm, after.length_mm);
            assert_eq!(before.status, after.status);
            assert_eq!(
                stable_profile(&world, before.profile),
                stable_profile(&candidate, after.profile)
            );
        }
        // 位姿批次按稳定引用逐点相等。
        assert_eq!(stable_pose_batch(&world), stable_pose_batch(&candidate));
        // 停车占用按稳定车位保持。
        let mapped_space = rebinding.parking_space(space_main).expect("retained space");
        assert_eq!(
            candidate.committed_parking_occupant(mapped_space),
            Some(parked)
        );
        // 候选在新根上继续确定性步进。
        let mut candidate = candidate;
        candidate
            .step(TickInput::new(100))
            .expect("candidate steps");
        assert_eq!(candidate.tick_index(), world.tick_index() + 1);
    }

    #[test]
    fn migrate_fails_closed_on_missing_edge_reference() {
        let mut world = installed_world(BASE, "fixture://migration-base");
        let rebinding_probe = revision(TARGET);
        let rebinding =
            CrossRevisionRebinding::build(world.revision.identity(), rebinding_probe.identity())
                .unwrap();
        let doomed = unmappable_edge(&world, &rebinding);
        let route = world
            .register_route(RouteRegisterInput::new(vec![doomed]))
            .expect("single-edge route");
        spawn_on(&mut world, route, 1_000, 5_000);
        assert_eq!(
            expect_migration_error(&world, revision(TARGET), &rebinding),
            CutoverError::UnmappableLaneEdge {
                base_edge: doomed.raw()
            }
        );
    }

    #[test]
    fn migrate_fails_closed_on_missing_parking_reference() {
        let mut world = installed_world(BASE, "fixture://migration-base");
        let (entry, exit) = entry_exit(&world);
        let route = world
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("route");
        let target_revision = revision(TARGET);
        let rebinding =
            CrossRevisionRebinding::build(world.revision.identity(), target_revision.identity())
                .unwrap();
        let (_, space_doomed) = parking_spaces(&world, &rebinding);
        world
            .spawn_parked_vehicle(
                ParkedVehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 1_000),
                ParkingTarget::ExplicitSpace(space_doomed),
            )
            .expect("park on doomed space");
        assert_eq!(
            expect_migration_error(&world, target_revision, &rebinding),
            CutoverError::UnmappableParkingSpace {
                base_space: space_doomed.raw()
            }
        );
    }

    #[test]
    fn migrate_fails_closed_on_shortened_edge_progress() {
        let mut world = installed_world(BASE, "fixture://migration-base");
        let (entry, exit) = entry_exit(&world);
        let route = world
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("route");
        // base 侧 entry 长 100 m，target 缩为 60 m：80 m 进度合法生成但重绑越界。
        let vehicle = spawn_on(&mut world, route, 80_000, 5_000);
        let target_revision = revision(TARGET);
        let rebinding =
            CrossRevisionRebinding::build(world.revision.identity(), target_revision.identity())
                .unwrap();
        assert_eq!(
            expect_migration_error(&world, target_revision, &rebinding),
            CutoverError::VehicleRevalidationFailed {
                vehicle: vehicle.index()
            }
        );
    }

    #[test]
    fn migrate_fails_closed_on_lowered_speed_limit() {
        let mut world = installed_world(BASE, "fixture://migration-base");
        let (entry, exit) = entry_exit(&world);
        let route = world
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("route");
        // base 限速 20 m/s，target 降为 8 m/s：15 m/s 合法生成但重绑超速。
        let vehicle = spawn_on(&mut world, route, 10_000, 15_000);
        let target_revision = revision(TARGET);
        let rebinding =
            CrossRevisionRebinding::build(world.revision.identity(), target_revision.identity())
                .unwrap();
        assert_eq!(
            expect_migration_error(&world, target_revision, &rebinding),
            CutoverError::VehicleRevalidationFailed {
                vehicle: vehicle.index()
            }
        );
    }

    #[test]
    fn migrate_fails_closed_on_newly_denied_suffix_access() {
        let mut world = installed_world(BASE, "fixture://migration-base");
        let (entry, exit) = entry_exit(&world);
        let route = world
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("route");
        let vehicle = spawn_on(&mut world, route, 10_000, 5_000);
        let target_revision = revision(TARGET);
        let rebinding =
            CrossRevisionRebinding::build(world.revision.identity(), target_revision.identity())
                .unwrap();
        assert_eq!(
            expect_migration_error(&world, target_revision, &rebinding),
            CutoverError::VehicleRevalidationFailed {
                vehicle: vehicle.index()
            }
        );
    }

    #[test]
    fn migrate_fails_closed_on_profile_length_drift() {
        // profile 漂移对：target 仅改 standard-car 车长（4.5 m → 6.0 m）。
        // 直移保留存量车长会与 save/restore 的派生态分歧（length 不入
        // 摘要、静默漂移），按不可映射整体失败关闭。
        let mut world = installed_world(PROFILE_BASE, "fixture://drift-base");
        let (entry, exit) = entry_exit(&world);
        let route = world
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("route");
        let vehicle = spawn_on(&mut world, route, 10_000, 5_000);
        let target_revision = revision(PROFILE_TARGET);
        let rebinding =
            CrossRevisionRebinding::build(world.revision.identity(), target_revision.identity())
                .unwrap();
        assert_eq!(
            expect_migration_error(&world, target_revision, &rebinding),
            CutoverError::ProfileDerivationMismatch {
                vehicle: vehicle.index()
            }
        );
    }

    #[test]
    fn migrate_fails_closed_on_profile_class_drift() {
        // class 漂移（手注入第二个类别序数模拟 target profile 改派类别）：
        // 直移保留存量类别会使迁移后世界的存档在恢复侧被
        // ProfileClassMismatch 拒绝，同样按不可映射失败关闭。
        let mut world = installed_world(BASE, "fixture://migration-base");
        let (entry, _exit) = entry_exit(&world);
        let route = world
            .register_route(RouteRegisterInput::new(vec![entry]))
            .expect("route");
        let vehicle = spawn_on(&mut world, route, 1_000, 5_000);
        let other_class = {
            let count = world
                .traffic()
                .entity_counts()
                .count(EntityKind::ParticipantClass);
            let current = world.vehicle_state(vehicle).expect("vehicle").class();
            (0..count)
                .map(laneflow_static_contract::ParticipantClassOrdinal::from_raw)
                .find(|class| *class != current)
                .expect("fixture exposes a second class")
        };
        let index = usize::try_from(vehicle.index()).expect("index");
        world.vehicles[index].state.as_mut().expect("vehicle").class = other_class;
        let target_revision = revision(TARGET);
        let rebinding =
            CrossRevisionRebinding::build(world.revision.identity(), target_revision.identity())
                .unwrap();
        assert_eq!(
            expect_migration_error(&world, target_revision, &rebinding),
            CutoverError::ProfileDerivationMismatch {
                vehicle: vehicle.index()
            }
        );
    }

    #[test]
    fn migrated_oracle_world_steps_identically_to_unmigrated() {
        let mut world = installed_world(ORACLE_BASE, "fixture://oracle-base");
        let (entry, exit) = entry_exit(&world);
        let route = world
            .register_route(RouteRegisterInput::new(vec![entry, exit]))
            .expect("route");
        spawn_on(&mut world, route, 20_000, 5_000);
        spawn_on(&mut world, route, 2_000, 5_000);
        for _ in 0..2 {
            world.step(TickInput::new(100)).expect("step");
        }
        let target_revision = revision(ORACLE_TARGET);
        let rebinding =
            CrossRevisionRebinding::build(world.revision.identity(), target_revision.identity())
                .unwrap();
        let target_origin = *target_revision.canonical_origin();
        let mut candidate = migrate_structural_clone(
            &world,
            Arc::clone(&target_revision),
            source_for(target_origin, "fixture://oracle-target"),
            &rebinding,
        )
        .expect("identity migration succeeds");
        assert_ne!(
            world.revision().canonical_origin().network_revision(),
            candidate.revision().canonical_origin().network_revision()
        );

        // 切换边界：已提交状态逐点一致（恒等 oracle，验收标准第二条）。
        let assert_same = |world: &TrafficWorld, candidate: &TrafficWorld| {
            assert_committed_logical_state_equal(world, candidate);
        };
        assert_same(&world, &candidate);
        // 继续步进仍逐点一致（交通语义段逐字节相等的直接推论）。
        for _ in 0..4 {
            world.step(TickInput::new(100)).expect("base step");
            candidate.step(TickInput::new(100)).expect("candidate step");
            assert_same(&world, &candidate);
        }
    }
}
