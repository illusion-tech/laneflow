//! 跨修订直移核心（#302 切换合同 §3；#513 切片 C-2）。
//!
//! LFSD 认证消费见 [`crate::cutover`] 的描述符方法。本模块交付另外两件：
//! base→target 稳定引用重绑表（两侧 `SharedIdentityIndex` 共同解析，LFSD
//! 绑定行完成两侧制品交叉验证后，映射权威即身份索引对）与结构克隆迁移
//! （候选与旧世界槽位布局一致——当期句柄恒等保持；逐实体重绑即重验证，
//! 任一实体引用不存在或原样重绑违反 target 不变量都整体失败关闭）。

use std::sync::Arc;

use laneflow_static_contract::{
    ConflictZoneOrdinal, EntityKind, EntityKindMarker, LaneEdgeOrdinal, ManeuverGateOrdinal,
    ManeuverPathOrdinal, Ordinal, OrdinalKind, ParkingFacilityOrdinal, ParkingSpaceOrdinal,
    ParticipantClassOrdinal, ParticipantStreamOrdinal, SignalAspect, VehicleProfileOrdinal,
    WaitingZoneOrdinal,
};
use laneflow_static_network::{ConflictPathAnchor, SharedIdentityIndex, SharedNetworkRevision};

use crate::migration_journal::VehicleDelta;
use crate::parking::ParkingRuntimeState;
use crate::tables::{RouteSlot, VehicleSlot, bodies_overlap, compile_route, route_access_denied};
use crate::{
    CommittedNetworkSource, CutoverError, ManeuverTraversalPhase, ManeuverTraversalState,
    ParkingBinding, ParkingReservation, ParkingSpaceState, ParkingTarget, RouteHandle,
    TrafficWorld, VehicleHandle, VehicleState, VehicleStatus, VirtualEntryAnchorSelector,
    WaitingMembership,
};

#[cfg(test)]
thread_local! {
    static STAGING_RESERVATIONS_BEFORE_FAILURE: core::cell::Cell<Option<usize>> =
        const { core::cell::Cell::new(None) };
    static CONFLICT_MIGRATION_CALLS: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_conflict_migration_calls() {
    CONFLICT_MIGRATION_CALLS.with(|calls| calls.set(0));
}

#[cfg(test)]
pub(crate) fn conflict_migration_calls() -> usize {
    CONFLICT_MIGRATION_CALLS.with(core::cell::Cell::get)
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

fn try_staging_vec<T>(capacity: usize) -> Result<Vec<T>, CutoverError> {
    let mut values = Vec::new();
    try_reserve_staging_exact(&mut values, capacity)?;
    Ok(values)
}

fn try_staging_slice<T: Default + Clone>(capacity: usize) -> Result<Box<[T]>, CutoverError> {
    let mut values = try_staging_vec(capacity)?;
    values.resize(capacity, T::default());
    Ok(values.into_boxed_slice())
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
    maneuver_paths: Vec<Option<ManeuverPathOrdinal>>,
    maneuver_gates: Vec<Option<ManeuverGateOrdinal>>,
    waiting_zones: Vec<Option<WaitingZoneOrdinal>>,
    participant_streams: Vec<Option<ParticipantStreamOrdinal>>,
    conflict_zones: Vec<Option<ConflictZoneOrdinal>>,
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
            maneuver_paths: map_kind(base, target, EntityKind::ManeuverPath)?,
            maneuver_gates: map_kind(base, target, EntityKind::ManeuverGate)?,
            waiting_zones: map_kind(base, target, EntityKind::WaitingZone)?,
            participant_streams: map_kind(base, target, EntityKind::ParticipantStream)?,
            conflict_zones: map_kind(base, target, EntityKind::ConflictZone)?,
        })
    }

    /// 释放四张重绑表（失败结算时归还内存；结算后事务不可再消费）。
    pub(crate) fn release(&mut self) {
        self.lane_edges = Vec::new();
        self.parking_facilities = Vec::new();
        self.parking_spaces = Vec::new();
        self.vehicle_profiles = Vec::new();
        self.participant_classes = Vec::new();
        self.maneuver_paths = Vec::new();
        self.maneuver_gates = Vec::new();
        self.waiting_zones = Vec::new();
        self.participant_streams = Vec::new();
        self.conflict_zones = Vec::new();
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

    #[must_use]
    pub(crate) fn maneuver_path(&self, base: ManeuverPathOrdinal) -> Option<ManeuverPathOrdinal> {
        self.maneuver_paths.get(base.index()).copied().flatten()
    }

    #[must_use]
    pub(crate) fn maneuver_gate(&self, base: ManeuverGateOrdinal) -> Option<ManeuverGateOrdinal> {
        self.maneuver_gates.get(base.index()).copied().flatten()
    }

    #[must_use]
    pub(crate) fn waiting_zone(&self, base: WaitingZoneOrdinal) -> Option<WaitingZoneOrdinal> {
        self.waiting_zones.get(base.index()).copied().flatten()
    }

    #[must_use]
    pub(crate) fn participant_stream(
        &self,
        base: ParticipantStreamOrdinal,
    ) -> Option<ParticipantStreamOrdinal> {
        self.participant_streams
            .get(base.index())
            .copied()
            .flatten()
    }

    #[must_use]
    pub(crate) fn conflict_zone(&self, base: ConflictZoneOrdinal) -> Option<ConflictZoneOrdinal> {
        self.conflict_zones.get(base.index()).copied().flatten()
    }
}

pub(crate) fn vehicle_state_from_delta(
    base_revision: &SharedNetworkRevision,
    candidate: &TrafficWorld,
    rebinding: &CrossRevisionRebinding,
    delta: &VehicleDelta,
) -> Result<VehicleState, CutoverError> {
    let invalid = || CutoverError::VehicleRevalidationFailed {
        vehicle: delta.slot,
    };
    let route = RouteHandle::new(delta.route_index, delta.route_generation);
    let compiled = candidate.compiled_route(route).ok_or_else(invalid)?;
    let mut traversal = None;
    if delta.traversal_present {
        let path = rebinding
            .maneuver_path(ManeuverPathOrdinal::from_raw(delta.maneuver_path))
            .ok_or_else(invalid)?;
        let gate = rebinding
            .maneuver_gate(ManeuverGateOrdinal::from_raw(delta.phase_gate))
            .ok_or_else(invalid)?;
        let anchor = candidate
            .resolve_maneuver_anchor(
                route,
                crate::world::ManeuverOccurrenceAnchor::EntryRouteEdgeIndex(
                    delta.maneuver_entry_route_edge_index,
                ),
                path,
                gate,
            )
            .ok_or_else(invalid)?;
        if delta.traversal_phase != 4
            && (delta.route_edge_index < anchor.entry_route_edge_index
                || delta.route_edge_index >= anchor.exit_route_edge_index)
        {
            return Err(invalid());
        }
        let phase_hop = anchor.gate_hop;
        let phase = match delta.traversal_phase {
            1 => Some(ManeuverTraversalPhase::PreGate {
                next_gate_hop: phase_hop,
            }),
            2 => Some(ManeuverTraversalPhase::Committed {
                last_crossed_gate_hop: phase_hop,
            }),
            3 => Some(ManeuverTraversalPhase::Waiting {
                release_gate_hop: phase_hop,
            }),
            // reservation 只属于 candidate ConflictArbiter；固定宽度 delta
            // 只重建 traversal 的 Gate 锚点，因此不会把 Clearing 误当成
            // “无 traversal”并触发 Waiting bootstrap。
            4 => Some(ManeuverTraversalPhase::Clearing {
                admission_gate_hop: phase_hop,
            }),
            _ => return Err(invalid()),
        };
        if let Some(phase) = phase {
            traversal = Some(ManeuverTraversalState {
                route,
                maneuver_occurrence_index: anchor.occurrence_index,
                phase,
            });
        }
    } else if delta.membership_present {
        return Err(invalid());
    }

    let mut membership = None;
    if delta.membership_present {
        let traversal = traversal.ok_or_else(invalid)?;
        let zone = rebinding
            .waiting_zone(WaitingZoneOrdinal::from_raw(delta.waiting_zone))
            .ok_or_else(invalid)?;
        let entry_gate = rebinding
            .maneuver_gate(ManeuverGateOrdinal::from_raw(delta.entry_gate))
            .ok_or_else(invalid)?;
        let release_gate = rebinding
            .maneuver_gate(ManeuverGateOrdinal::from_raw(delta.release_gate))
            .ok_or_else(invalid)?;
        let occurrence = compiled
            .waiting
            .iter()
            .find(|occurrence| {
                occurrence.maneuver_index == traversal.maneuver_occurrence_index
                    && occurrence.zone == zone
                    && compiled
                        .hop_gate
                        .get(occurrence.entry_hop as usize)
                        .copied()
                        .flatten()
                        == Some(entry_gate)
                    && compiled
                        .hop_gate
                        .get(occurrence.release_hop as usize)
                        .copied()
                        .flatten()
                        == Some(release_gate)
            })
            .ok_or_else(invalid)?;
        membership = Some(WaitingMembership {
            waiting_zone: zone,
            admission_sequence: delta.admission_sequence,
            release_hop: occurrence.release_hop,
        });
    }

    let mut state = VehicleState {
        handle: VehicleHandle::new(delta.slot, delta.generation),
        profile: rebinding
            .vehicle_profile(VehicleProfileOrdinal::from_raw(delta.profile))
            .ok_or(CutoverError::UnmappableVehicleProfile {
                base_profile: delta.profile,
            })?,
        class: rebinding
            .participant_class(ParticipantClassOrdinal::from_raw(delta.class))
            .ok_or(CutoverError::UnmappableParticipantClass {
                base_class: delta.class,
            })?,
        route,
        route_edge_index: delta.route_edge_index,
        progress_mm: delta.progress_mm,
        carry_um: delta.carry_um,
        speed_mm_s: delta.speed_mm_s,
        length_mm: delta.length_mm,
        status: delta.status,
        maneuver_traversal: traversal,
        waiting_membership: membership,
    };
    if state.status == VehicleStatus::Active && state.maneuver_traversal.is_none() {
        let bootstrap = candidate
            .derive_waiting_traversal_with_signals(state, false)
            .map_err(|_| invalid())?;
        if let Some(traversal) = bootstrap {
            if !matches!(traversal.phase, ManeuverTraversalPhase::PreGate { .. }) {
                return Err(invalid());
            }
            let path = compiled.maneuvers[traversal.maneuver_occurrence_index as usize].path;
            let stable_path = candidate
                .binding
                .revision
                .identity()
                .stable_id(path)
                .ok_or_else(invalid)?;
            let base_path = base_revision
                .identity()
                .ordinal(stable_path)
                .ok_or_else(invalid)?;
            if !base_revision
                .traffic()
                .maneuvers()
                .maneuver_path(base_path)
                .ok_or_else(invalid)?
                .waiting_zones()
                .is_empty()
            {
                return Err(invalid());
            }
            // 仅首次覆盖、尚未跨第一个 Gate 的零历史初始化；bootstrap 不生成 membership。
            state.maneuver_traversal = Some(traversal);
        }
    }
    if !candidate.restored_waiting_authority_valid(state) {
        return Err(invalid());
    }
    Ok(state)
}

fn try_clone<T: Clone>(source: &[T]) -> Result<Vec<T>, CutoverError> {
    let mut clone = Vec::new();
    clone
        .try_reserve_exact(source.len())
        .map_err(|_| CutoverError::StagingAllocFailed)?;
    clone.extend_from_slice(source);
    Ok(clone)
}

fn waiting_occurrences_rebind(
    base: &crate::tables::CompiledRoute,
    target: &crate::tables::CompiledRoute,
    rebinding: &CrossRevisionRebinding,
) -> bool {
    base.waiting.iter().all(|occurrence| {
        let Some(base_maneuver) = base.maneuvers.get(occurrence.maneuver_index as usize) else {
            return false;
        };
        let Some(target_path) = rebinding.maneuver_path(base_maneuver.path) else {
            return false;
        };
        let Some(target_zone) = rebinding.waiting_zone(occurrence.zone) else {
            return false;
        };
        let Some(target_entry_gate) = base
            .hop_gate
            .get(occurrence.entry_hop as usize)
            .copied()
            .flatten()
            .and_then(|gate| rebinding.maneuver_gate(gate))
        else {
            return false;
        };
        let Some(target_release_gate) = base
            .hop_gate
            .get(occurrence.release_hop as usize)
            .copied()
            .flatten()
            .and_then(|gate| rebinding.maneuver_gate(gate))
        else {
            return false;
        };
        target.waiting.iter().any(|candidate| {
            candidate.entry_hop == occurrence.entry_hop
                && candidate.release_hop == occurrence.release_hop
                && candidate.zone == target_zone
                && target
                    .maneuvers
                    .get(candidate.maneuver_index as usize)
                    .is_some_and(|maneuver| maneuver.path == target_path)
                && target
                    .hop_gate
                    .get(candidate.entry_hop as usize)
                    .copied()
                    .flatten()
                    == Some(target_entry_gate)
                && target
                    .hop_gate
                    .get(candidate.release_hop as usize)
                    .copied()
                    .flatten()
                    == Some(target_release_gate)
        })
    })
}

pub(crate) fn revalidate_waiting_routes(
    base: &TrafficWorld,
    target: &TrafficWorld,
    rebinding: &CrossRevisionRebinding,
) -> Result<(), CutoverError> {
    if base.committed.routes.len() != target.committed.routes.len() {
        return Err(CutoverError::ReplayInconsistent);
    }
    for (base_slot, target_slot) in base.committed.routes.iter().zip(&target.committed.routes) {
        if base_slot.generation != target_slot.generation {
            return Err(CutoverError::ReplayInconsistent);
        }
        match (base_slot.compiled.as_ref(), target_slot.compiled.as_ref()) {
            (Some(base_route), Some(target_route))
                if waiting_occurrences_rebind(base_route, target_route, rebinding) => {}
            (None, None) => {}
            (Some(_), Some(_)) => return Err(CutoverError::WaitingRevalidationFailed),
            _ => return Err(CutoverError::ReplayInconsistent),
        }
    }
    Ok(())
}

/// 把 `world` 的动态状态结构克隆到 `target_revision` 上并完成直移。
///
/// 候选与旧世界槽位布局一致（当期 `RouteHandle` / `VehicleHandle` 恒等保持，
/// 切换合同 §3 逻辑恒等）；tick/时间/游标保持克隆时点的基线值。逐实体
/// 重绑即重验证：任一引用不存在、路线重编译失败或车辆原样重绑违反
/// target 不变量（进度越界、超速、后缀访问被拒、与其它迁移车辆重叠）都
/// 返回错误并丢弃候选——旧世界从不被本函数触及。
#[cfg(test)]
pub(crate) fn migrate_structural_clone(
    world: &TrafficWorld,
    target_revision: Arc<SharedNetworkRevision>,
    target_source: CommittedNetworkSource,
    rebinding: &CrossRevisionRebinding,
) -> Result<TrafficWorld, CutoverError> {
    migrate_structural_clone_with_conflict_plan(world, target_revision, target_source, rebinding)
        .map(|(candidate, _)| candidate)
}

pub(crate) fn migrate_structural_clone_with_conflict_plan(
    world: &TrafficWorld,
    target_revision: Arc<SharedNetworkRevision>,
    target_source: CommittedNetworkSource,
    rebinding: &CrossRevisionRebinding,
) -> Result<(TrafficWorld, ConflictCutoverFinalizationPlan), CutoverError> {
    world.validate_cutover_policy(&target_revision)?;
    let policy_binding = crate::policy::WorldPolicyBinding::install(
        &target_revision,
        world.policy_selection(),
        world.binding.config.fixed_delta_time_ms(),
    )
    .map_err(CutoverError::PolicyInstall)?;
    let target_traffic = target_revision.traffic();

    // 路线：逐槽位重绑边序数并对 target 根重编译（等价重执行 register_route
    // 的编译检查）。
    let mut routes = Vec::new();
    routes
        .try_reserve_exact(world.committed.routes.len())
        .map_err(|_| CutoverError::StagingAllocFailed)?;
    let mut conflict_occurrence_total = 0_u64;
    for slot in &world.committed.routes {
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
            world.binding.config.route_conflict_occurrence_capacity(),
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
        if !waiting_occurrences_rebind(compiled, &migrated, rebinding) {
            return Err(CutoverError::WaitingRevalidationFailed);
        }
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
        .try_reserve_exact(world.committed.vehicles.len())
        .map_err(|_| CutoverError::StagingAllocFailed)?;
    for slot in &world.committed.vehicles {
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
        migrated.maneuver_traversal = None;
        migrated.waiting_membership = None;
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
    for vehicle in world.committed.live_order.iter().copied() {
        let Some(binding) = world.committed.parking.binding(vehicle) else {
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
                            .binding
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

    let mut free_routes = try_clone(world.committed.free_routes.as_slice())?;
    let mut free_vehicles = try_clone(world.committed.free_vehicles.as_slice())?;
    let mut live_order = try_clone(world.committed.live_order.as_slice())?;
    let mut active_order = try_clone(world.derived.active_order.as_slice())?;
    // install 同构容量余量：晋升后的世界在配置容量内的生命周期命令不触发
    // 无检分配；窗口重放的 push 同界（上游注册已受容量约束）。
    let route_capacity = usize::try_from(world.binding.config.route_capacity()).unwrap_or(0);
    let vehicle_capacity = usize::try_from(world.binding.config.vehicle_capacity()).unwrap_or(0);
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
        .try_reserve_exact(world.workspace.next_states.capacity())
        .map_err(|_| CutoverError::StagingAllocFailed)?;
    let mut signal_aspects = Vec::new();
    try_reserve_staging_exact(&mut signal_aspects, group_count)?;
    signal_aspects.resize(group_count, SignalAspect::Red);
    let mut next_signal_aspects = Vec::new();
    try_reserve_staging_exact(&mut next_signal_aspects, group_count)?;
    next_signal_aspects.resize(group_count, SignalAspect::Red);
    let waiting_zone_count = usize::try_from(
        target_revision
            .traffic()
            .entity_counts()
            .count(laneflow_static_contract::EntityKind::WaitingZone),
    )
    .expect("waiting zone count fits usize");
    let conflict_arbiter =
        crate::conflict::ConflictArbiter::install(&target_revision, vehicle_capacity)
            .map_err(|_| CutoverError::StagingAllocFailed)?;

    // occurrence 总数重算（迁移不增减边数；防御性闭合后文校验容量）。
    let mut occurrence_total: u64 = 0;
    for slot in &routes {
        if let Some(compiled) = slot.compiled.as_ref() {
            occurrence_total +=
                u64::try_from(compiled.edges.len()).expect("route edge count fits u64");
        }
    }

    let revision = target_revision;
    let source = target_source;
    let world_id = world.binding.world_id;
    let world_generation = world.binding.world_generation;
    let config = world.binding.config;
    let (conflict, conflict_indexes, conflict_workspace) = conflict_arbiter.into_parts();
    let conflict_eligibility = try_staging_vec(vehicle_capacity)?;
    let conflict_candidates = try_staging_vec(vehicle_capacity)?;
    let conflict_schedule = crate::conflict_tick::ConflictSchedule::default();
    let conflict_candidate_cells = Vec::new();
    let conflict_candidate_downstream = Vec::new();
    let conflict_cell_work = Vec::new();
    let conflict_downstream_work = Vec::new();
    let conflict_grants = try_staging_vec(vehicle_capacity)?;
    let conflict_motion_by_vehicle = try_staging_slice(vehicle_capacity)?;
    let conflict_next_eligibility = try_staging_slice(vehicle_capacity)?;
    let conflict_passage_transitions = Vec::new();
    let conflict_changed_owners = try_staging_vec(vehicle_capacity)?;
    let waiting_dependencies = crate::waiting_dependencies::WaitingDependencies::default();
    let conflict_staged_decisions = try_staging_vec(vehicle_capacity)?;
    let latest_conflict_decisions = try_staging_vec(vehicle_capacity)?;
    let tick_index = world.committed.tick_index;
    let time_ms = world.committed.time_ms;
    let command_cursor = world.committed.command_cursor;
    let event_cursor = world.committed.event_cursor;
    let observation_state_sequence = world.committed.observation_state_sequence;
    let signal_aspects = signal_aspects.into_boxed_slice();
    let next_signal_aspects = next_signal_aspects.into_boxed_slice();
    let live_route_count = world.committed.live_route_count;
    let live_route_edge_occurrence_count = occurrence_total;
    let live_route_conflict_occurrence_count = conflict_occurrence_total;
    let waiting_zones = try_staging_slice(waiting_zone_count)?;
    let waiting_queue_ends = try_staging_slice(waiting_zone_count)?;
    let waiting_links = try_staging_slice(vehicle_capacity)?;
    let waiting_member_rows = try_staging_vec(vehicle_capacity)?;
    let waiting_claims = try_staging_vec(vehicle_capacity)?;
    let waiting_plans = try_staging_vec(vehicle_capacity)?;
    let waiting_plan_by_vehicle = try_staging_slice(vehicle_capacity)?;
    let next_state_by_vehicle = try_staging_slice(vehicle_capacity)?;
    let waiting_staged_decisions = Vec::new();
    let staged_transition_events = Vec::new();
    let waiting_next_counters = try_staging_slice(waiting_zone_count)?;
    let waiting_staged_occupancy = try_staging_slice(waiting_zone_count)?;
    let waiting_staged_storage_mm = try_staging_slice(waiting_zone_count)?;
    let latest_waiting_decisions = Vec::new();
    let latest_transition_events = Vec::new();
    let (occupancy, occupancy_scratch) = crate::occupancy::OccupancyIndex::with_capacity(0, 0);
    let migration_journal = None;
    let migration_epoch = 0;
    let mut candidate = TrafficWorld {
        binding: crate::state::WorldBindingState {
            policy_binding,
            revision,
            source,
            world_id,
            world_generation,
            config,
        },
        committed: crate::state::CommittedWorldState {
            conflict,
            conflict_eligibility,
            latest_conflict_decisions,
            tick_index,
            time_ms,
            command_cursor,
            event_cursor,
            observation_state_sequence,
            signal_aspects,
            routes,
            free_routes,
            live_route_count,
            live_route_edge_occurrence_count,
            live_route_conflict_occurrence_count,
            vehicles,
            free_vehicles,
            live_order,
            parking,
            waiting_zones,
            latest_waiting_decisions,
            latest_transition_events,
        },
        derived: crate::state::DerivedIndexes {
            conflict: conflict_indexes,
            active_order,
            waiting_queue_ends,
            waiting_links,
            waiting_member_rows,
            occupancy,
        },
        workspace: crate::state::TickWorkspace {
            conflict: conflict_workspace,
            conflict_candidates,
            conflict_schedule,
            conflict_candidate_cells,
            conflict_candidate_downstream,
            conflict_cell_work,
            conflict_downstream_work,
            conflict_grants,
            conflict_motion_by_vehicle,
            conflict_next_eligibility,
            conflict_passage_transitions,
            conflict_changed_owners,
            waiting_dependencies,
            conflict_staged_decisions,
            next_signal_aspects,
            waiting_claims,
            waiting_plans,
            waiting_plan_by_vehicle,
            next_state_by_vehicle,
            waiting_staged_decisions,
            staged_transition_events,
            waiting_next_counters,
            waiting_staged_occupancy,
            waiting_staged_storage_mm,
            occupancy_scratch,
            next_states,
        },
        admin: crate::state::AdministrativeState {
            migration_journal,
            migration_epoch,
        },
    };
    if candidate.committed.live_route_edge_occurrence_count
        > world.binding.config.route_edge_occurrence_capacity()
    {
        return Err(CutoverError::EdgeOccurrenceCapacityExceeded {
            total: candidate.committed.live_route_edge_occurrence_count,
            capacity: world.binding.config.route_edge_occurrence_capacity(),
        });
    }
    candidate.refresh_signals();
    for (base_index, base_zone) in world.committed.waiting_zones.iter().copied().enumerate() {
        let base = WaitingZoneOrdinal::from_raw(
            u32::try_from(base_index).expect("WaitingZone index fits u32"),
        );
        match rebinding.waiting_zone(base) {
            Some(target) => {
                candidate.committed.waiting_zones[target.index()].next_admission_sequence =
                    base_zone.next_admission_sequence;
            }
            None if base_zone.next_admission_sequence == 0 => {}
            None => return Err(CutoverError::WaitingRevalidationFailed),
        }
    }
    for handle in world.committed.live_order.iter().copied() {
        let base_state = world
            .vehicle_state(handle)
            .ok_or(CutoverError::WaitingRevalidationFailed)?;
        let delta = VehicleDelta::from_state(base_state, world.compiled_route(base_state.route));
        let migrated =
            vehicle_state_from_delta(&world.binding.revision, &candidate, rebinding, &delta)?;
        candidate.committed.vehicles[handle.index() as usize].state = Some(migrated);
    }
    if !candidate.rebuild_waiting_aggregate_from_semantics() {
        return Err(CutoverError::WaitingRevalidationFailed);
    }
    let conflict_finalization =
        migrate_conflict_state(world, &mut candidate, rebinding, world.committed.time_ms)?;
    revalidate_migrated_vehicles(&mut candidate)?;
    Ok((candidate, conflict_finalization))
}

fn mapped_conflict_address(
    _source: &TrafficWorld,
    target: &TrafficWorld,
    rebinding: &CrossRevisionRebinding,
    source_address: crate::ConflictPassageAddress,
) -> Option<crate::ConflictPassageAddress> {
    let stream = rebinding.participant_stream(source_address.stream())?;
    let zone = rebinding.conflict_zone(source_address.zone())?;
    target.conflict_read().unique_address(zone, stream)
}

fn conflict_anchor_rebinds(
    source: ConflictPathAnchor,
    target: ConflictPathAnchor,
    rebinding: &CrossRevisionRebinding,
) -> bool {
    match (source, target) {
        (ConflictPathAnchor::Gate(source), ConflictPathAnchor::Gate(target)) => {
            rebinding.maneuver_gate(source) == Some(target)
        }
        (ConflictPathAnchor::EdgeBoundary(source), ConflictPathAnchor::EdgeBoundary(target)) => {
            source == target
        }
        (
            ConflictPathAnchor::Interior {
                path_edge_index: source_index,
                progress_millimetres: source_progress,
            },
            ConflictPathAnchor::Interior {
                path_edge_index: target_index,
                progress_millimetres: target_progress,
            },
        ) => source_index == target_index && source_progress == target_progress,
        _ => false,
    }
}

fn conflict_anchor_path_cursor(
    revision: &SharedNetworkRevision,
    path: ManeuverPathOrdinal,
    anchor: ConflictPathAnchor,
) -> Option<(usize, u32)> {
    let path_view = revision.traffic().maneuvers().maneuver_path(path)?;
    match anchor {
        ConflictPathAnchor::Gate(gate) => {
            let transition = revision
                .traffic()
                .relations()
                .maneuver_gate(gate)
                .filter(|gate| gate.path() == path)?
                .transition_index();
            let boundary = usize::try_from(transition).ok()?.checked_add(1)?;
            (boundary <= path_view.edges().len()).then_some((boundary, 0))
        }
        ConflictPathAnchor::EdgeBoundary(boundary) => {
            let boundary = usize::try_from(boundary).ok()?;
            (boundary <= path_view.edges().len()).then_some((boundary, 0))
        }
        ConflictPathAnchor::Interior {
            path_edge_index,
            progress_millimetres,
        } => {
            let index = usize::try_from(path_edge_index).ok()?;
            let edge = *path_view.edges().get(index)?;
            let length = *revision
                .traffic()
                .lane_lengths_millimetres()
                .get(edge.index())?;
            (progress_millimetres > 0 && progress_millimetres < length)
                .then_some((index, progress_millimetres))
        }
    }
}

fn conflict_passage_physical_interval_continuous(
    source: &SharedNetworkRevision,
    target: &SharedNetworkRevision,
    source_path: ManeuverPathOrdinal,
    target_path: ManeuverPathOrdinal,
    source_passage: laneflow_static_network::ConflictPassage,
    target_passage: laneflow_static_network::ConflictPassage,
) -> bool {
    let Some(source_path_view) = source.traffic().maneuvers().maneuver_path(source_path) else {
        return false;
    };
    let Some(target_path_view) = target.traffic().maneuvers().maneuver_path(target_path) else {
        return false;
    };
    let Some(source_start) =
        conflict_anchor_path_cursor(source, source_path, source_passage.entry())
    else {
        return false;
    };
    let Some(source_end) = conflict_anchor_path_cursor(source, source_path, source_passage.exit())
    else {
        return false;
    };
    let Some(target_start) =
        conflict_anchor_path_cursor(target, target_path, target_passage.entry())
    else {
        return false;
    };
    let Some(target_end) = conflict_anchor_path_cursor(target, target_path, target_passage.exit())
    else {
        return false;
    };
    if source_start != target_start
        || source_end != target_end
        || source_start >= source_end
        || source_path_view.edges().len() != target_path_view.edges().len()
    {
        return false;
    }
    let source_lengths = source.traffic().lane_lengths_millimetres();
    let target_lengths = target.traffic().lane_lengths_millimetres();
    let last = if source_end.1 == 0 {
        source_end.0
    } else {
        source_end.0.saturating_add(1)
    };
    for index in source_start.0..last {
        let Some(source_edge) = source_path_view.edges().get(index).copied() else {
            return false;
        };
        let Some(target_edge) = target_path_view.edges().get(index).copied() else {
            return false;
        };
        let Some(source_length) = source_lengths.get(source_edge.index()).copied() else {
            return false;
        };
        let Some(target_length) = target_lengths.get(target_edge.index()).copied() else {
            return false;
        };
        let start = if index == source_start.0 {
            source_start.1
        } else {
            0
        };
        let source_end_mm = if index == source_end.0 && source_end.1 != 0 {
            source_end.1
        } else {
            source_length
        };
        let target_end_mm = if index == target_end.0 && target_end.1 != 0 {
            target_end.1
        } else {
            target_length
        };
        if start >= source_end_mm || start >= target_end_mm || source_end_mm != target_end_mm {
            return false;
        }
    }
    true
}

pub(crate) fn conflict_passage_semantics_continuous(
    source: &SharedNetworkRevision,
    target: &SharedNetworkRevision,
    rebinding: &CrossRevisionRebinding,
    source_address: crate::ConflictPassageAddress,
    target_address: crate::ConflictPassageAddress,
) -> bool {
    let Some(source_stream) = source
        .conflict()
        .participant_stream(source_address.stream())
    else {
        return false;
    };
    let Some(target_stream) = target
        .conflict()
        .participant_stream(target_address.stream())
    else {
        return false;
    };
    let Some(source_passage) = source_stream
        .passages()
        .get(source_address.passage_local_index() as usize)
        .copied()
    else {
        return false;
    };
    let Some(target_passage) = target_stream
        .passages()
        .get(target_address.passage_local_index() as usize)
        .copied()
    else {
        return false;
    };
    let Some(target_path) = rebinding.maneuver_path(source_stream.maneuver_path()) else {
        return false;
    };
    if target_stream.maneuver_path() != target_path
        || rebinding.maneuver_gate(source_passage.admission_gate())
            != Some(target_passage.admission_gate())
        || source.identity().stable_id(source_stream.junction())
            != target.identity().stable_id(target_stream.junction())
    {
        return false;
    }
    let Some(source_path) = source
        .traffic()
        .maneuvers()
        .maneuver_path(source_stream.maneuver_path())
    else {
        return false;
    };
    let Some(target_path_view) = target.traffic().maneuvers().maneuver_path(target_path) else {
        return false;
    };
    source_path.edges().len() == target_path_view.edges().len()
        && source_path
            .edges()
            .iter()
            .zip(target_path_view.edges())
            .all(|(source, target)| rebinding.lane_edge(*source) == Some(*target))
        && conflict_anchor_rebinds(source_passage.entry(), target_passage.entry(), rebinding)
        && conflict_anchor_rebinds(source_passage.exit(), target_passage.exit(), rebinding)
        && conflict_passage_physical_interval_continuous(
            source,
            target,
            source_stream.maneuver_path(),
            target_path,
            source_passage,
            target_passage,
        )
}

fn mapped_conflict_occurrence(
    source: &TrafficWorld,
    target: &TrafficWorld,
    rebinding: &CrossRevisionRebinding,
    source_route: RouteHandle,
    target_route: RouteHandle,
    source_occurrence_index: u32,
) -> Option<(u32, crate::ConflictPassageAddress)> {
    let source_compiled = source.compiled_route(source_route)?;
    let target_compiled = target.compiled_route(target_route)?;
    let source_occurrence = *source_compiled
        .conflicts
        .get(source_occurrence_index as usize)?;
    let source_address = source_occurrence.address();
    let target_address = mapped_conflict_address(source, target, rebinding, source_address)?;
    if !conflict_passage_semantics_continuous(
        &source.binding.revision,
        &target.binding.revision,
        rebinding,
        source_address,
        target_address,
    ) {
        return None;
    }
    let source_maneuver = *source_compiled
        .maneuvers
        .get(source_occurrence.maneuver_index as usize)?;
    let target_path = rebinding.maneuver_path(source_maneuver.path)?;
    let mut target_maneuvers =
        target_compiled
            .maneuvers
            .iter()
            .enumerate()
            .filter(|(_, maneuver)| {
                maneuver.path == target_path
                    && maneuver.entry_route_edge_index == source_maneuver.entry_route_edge_index
            });
    let (target_maneuver_index, _) = target_maneuvers.next()?;
    if target_maneuvers.next().is_some() {
        return None;
    }
    let target_maneuver_index = u32::try_from(target_maneuver_index).ok()?;
    let mut matches = target_compiled
        .conflicts
        .iter()
        .enumerate()
        .filter(|(_, occurrence)| {
            occurrence.maneuver_index == target_maneuver_index
                && occurrence.address() == target_address
                && occurrence.entry == source_occurrence.entry
                && occurrence.clearance == source_occurrence.clearance
        });
    let (target_occurrence_index, _) = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some((u32::try_from(target_occurrence_index).ok()?, target_address))
}

fn cutover_route_position_um(
    world: &TrafficWorld,
    route: RouteHandle,
    route_edge_index: u32,
    progress_mm: u32,
    carry_um: u16,
) -> Option<u128> {
    if carry_um >= 1_000 {
        return None;
    }
    let edges = world.route_edges(route)?;
    let index = usize::try_from(route_edge_index).ok()?;
    let edge = *edges.get(index)?;
    let lengths = world.binding.revision.traffic().lane_lengths_millimetres();
    if progress_mm > *lengths.get(edge.index())? {
        return None;
    }
    let prefix = edges[..index].iter().try_fold(0_u128, |sum, edge| {
        sum.checked_add(u128::from(*lengths.get(edge.index())?))
    })?;
    prefix
        .checked_add(u128::from(progress_mm))?
        .checked_mul(1_000)?
        .checked_add(u128::from(carry_um))
}

/// Prepare 已验证的 Conflict 静默点最终化计划。
#[derive(Debug, Default)]
pub(crate) struct ConflictCutoverFinalizationPlan {
    floor_addresses: Vec<crate::ConflictPassageAddress>,
    removed_history_ready_at_ms: Option<u64>,
}

/// 从完整来源权威重建候选 Conflict 状态。Prepare 返回新/不连续 cell 的
/// 精确地址与删除历史到期计划；静默提交只校验/最终化该计划，不再全量重迁移。
pub(crate) fn migrate_conflict_state(
    source: &TrafficWorld,
    target: &mut TrafficWorld,
    rebinding: &CrossRevisionRebinding,
    prepare_time_ms: u64,
) -> Result<ConflictCutoverFinalizationPlan, CutoverError> {
    #[cfg(test)]
    CONFLICT_MIGRATION_CALLS.with(|calls| calls.set(calls.get() + 1));
    if prepare_time_ms != source.committed.time_ms {
        return Err(CutoverError::ConflictRevalidationFailed);
    }
    let vehicle_capacity = usize::try_from(target.binding.config.vehicle_capacity())
        .map_err(|_| CutoverError::StagingAllocFailed)?;
    let mut arbiter =
        crate::conflict::ConflictArbiter::install(&target.binding.revision, vehicle_capacity)
            .map_err(|_| CutoverError::StagingAllocFailed)?;
    let mut eligibility =
        try_staging_slice::<Option<crate::ConflictEligibilityState>>(vehicle_capacity)?;
    let source_vehicle_capacity = usize::try_from(source.binding.config.vehicle_capacity())
        .map_err(|_| CutoverError::StagingAllocFailed)?;
    let mut source_conflict_index = try_staging_slice::<
        Option<crate::conflict::CommittedConflictIndexEntry>,
    >(source_vehicle_capacity)?;
    let source_conflict_view = source
        .conflict_read()
        .persistence_view(&mut source_conflict_index)
        .ok_or(CutoverError::ConflictRevalidationFailed)?;
    let mut restored_traversals: Vec<(VehicleHandle, ManeuverTraversalState)> =
        try_staging_vec(source.committed.live_order.len())?;

    for handle in source.committed.live_order.iter().copied() {
        let source_state = source
            .vehicle_state(handle)
            .ok_or(CutoverError::ConflictRevalidationFailed)?;
        let target_state = *target
            .vehicle_state(handle)
            .ok_or(CutoverError::ConflictRevalidationFailed)?;
        if let Some(source_eligibility) = source
            .committed
            .conflict_eligibility
            .get(handle.index() as usize)
            .copied()
            .flatten()
        {
            if source_conflict_view.authority(handle).is_some()
                || target_state.status != VehicleStatus::Active
            {
                return Err(CutoverError::ConflictRevalidationFailed);
            }
            let source_locator = source_eligibility.locator();
            let target_locator = mapped_conflict_occurrence(
                source,
                target,
                rebinding,
                source_state.route,
                target_state.route,
                source_locator.conflict_occurrence_index(),
            )
            .and_then(|(target_occurrence, _)| {
                target.conflict_passage_occurrence_locator(target_state.route, target_occurrence)
            });
            eligibility[handle.index() as usize] = target_locator.and_then(|target_locator| {
                let migrated = crate::ConflictEligibilityState::update(
                    None,
                    target_locator,
                    true,
                    source_eligibility.first_eligible_tick(),
                )
                .expect("true predicate creates eligibility");
                target
                    .conflict_eligibility_authority_valid(&target_state, migrated)
                    .then_some(migrated)
            });
        }

        let Some(source_authority) = source_conflict_view.authority(handle) else {
            continue;
        };
        let source_reservation = source_authority.reservation;
        if target_state.status != VehicleStatus::Active
            || target_state.waiting_membership.is_some()
            || source_reservation.acquired_tick() > source.committed.tick_index
        {
            return Err(CutoverError::ConflictRevalidationFailed);
        }
        let source_range = source_reservation.passage_range();
        let end = source_range
            .first_conflict_occurrence_index()
            .checked_add(source_range.passage_count())
            .ok_or(CutoverError::ConflictRevalidationFailed)?;
        let mut target_occurrences: Vec<(u32, crate::ConflictPassageAddress)> =
            try_staging_vec(source_range.passage_count() as usize)?;
        for source_occurrence in source_range.first_conflict_occurrence_index()..end {
            target_occurrences.push(
                mapped_conflict_occurrence(
                    source,
                    target,
                    rebinding,
                    source_state.route,
                    target_state.route,
                    source_occurrence,
                )
                .ok_or(CutoverError::ConflictRevalidationFailed)?,
            );
        }
        target_occurrences.sort_unstable_by_key(|(index, _)| *index);
        let first_target_occurrence = target_occurrences
            .first()
            .map(|(index, _)| *index)
            .ok_or(CutoverError::ConflictRevalidationFailed)?;
        if target_occurrences
            .iter()
            .enumerate()
            .any(|(offset, (index, _))| {
                first_target_occurrence.checked_add(offset as u32) != Some(*index)
            })
        {
            return Err(CutoverError::ConflictRevalidationFailed);
        }
        let target_compiled = target
            .compiled_route(target_state.route)
            .ok_or(CutoverError::ConflictRevalidationFailed)?;
        let first_static = target_compiled
            .conflicts
            .get(first_target_occurrence as usize)
            .ok_or(CutoverError::ConflictRevalidationFailed)?;
        if target_occurrences.iter().any(|(index, _)| {
            target_compiled
                .conflicts
                .get(*index as usize)
                .is_none_or(|occurrence| {
                    occurrence.maneuver_index != first_static.maneuver_index
                        || occurrence.admission_hop != first_static.admission_hop
                })
        }) {
            return Err(CutoverError::ConflictRevalidationFailed);
        }
        let target_range = crate::ConflictPassageRange::new(
            target_state.route,
            first_static.maneuver_index,
            first_static.admission_hop,
            first_target_occurrence,
            u32::try_from(target_occurrences.len())
                .map_err(|_| CutoverError::ConflictRevalidationFailed)?,
        )
        .ok_or(CutoverError::ConflictRevalidationFailed)?;
        let front_um = cutover_route_position_um(
            target,
            target_state.route,
            target_state.route_edge_index,
            target_state.progress_mm,
            target_state.carry_um,
        )
        .ok_or(CutoverError::ConflictRevalidationFailed)?;
        let tail_um = i128::try_from(front_um)
            .map_err(|_| CutoverError::ConflictRevalidationFailed)?
            - i128::from(target_state.length_mm) * 1_000;
        let mut cells = try_staging_vec(target_occurrences.len())?;
        for (target_occurrence, address) in &target_occurrences {
            let occurrence = target_compiled
                .conflicts
                .get(*target_occurrence as usize)
                .ok_or(CutoverError::ConflictRevalidationFailed)?;
            let entry_um = cutover_route_position_um(
                target,
                target_state.route,
                occurrence.entry.route_edge_index,
                occurrence.entry.progress_mm,
                0,
            )
            .ok_or(CutoverError::ConflictRevalidationFailed)?;
            let clearance_um = cutover_route_position_um(
                target,
                target_state.route,
                occurrence.clearance.route_edge_index,
                occurrence.clearance.progress_mm,
                0,
            )
            .ok_or(CutoverError::ConflictRevalidationFailed)?;
            let cleared = tail_um
                >= i128::try_from(clearance_um)
                    .map_err(|_| CutoverError::ConflictRevalidationFailed)?;
            cells.push(crate::conflict::RestoredConflictCell {
                address: *address,
                occupant: front_um >= entry_um && !cleared,
                cleared,
            });
        }
        cells.sort_unstable_by_key(|cell| cell.address);

        let claim_count = source_reservation.downstream_claim_count() as usize;
        let source_gap = source
            .binding
            .revision
            .traffic()
            .relations()
            .vehicle_profile(source_state.profile)
            .map(|profile| profile.min_gap_mm())
            .ok_or(CutoverError::ConflictRevalidationFailed)?;
        let mut source_downstream = try_staging_vec(claim_count)?;
        for claim in source_authority.downstream_claims() {
            if claim.follower_min_gap_mm != source_gap {
                return Err(CutoverError::ConflictRevalidationFailed);
            }
            source_downstream.push(claim.interval);
        }
        let source_downstream_plan = source
            .reservation_downstream_claim_plan(source_range, source_state.length_mm)
            .map_err(|_| CutoverError::ConflictRevalidationFailed)?;
        let mut expected_source = try_staging_vec(source_downstream_plan.raw_interval_capacity())?;
        source
            .derive_reservation_downstream_claims_from_plan(
                source_downstream_plan,
                &mut expected_source,
            )
            .map_err(|_| CutoverError::ConflictRevalidationFailed)?;
        if source_downstream != expected_source {
            return Err(CutoverError::ConflictRevalidationFailed);
        }

        let mut mapped_source = try_staging_vec(claim_count)?;
        for claim in source_authority.downstream_claims() {
            let target_edge = rebinding
                .lane_edge(claim.interval.edge())
                .ok_or(CutoverError::ConflictRevalidationFailed)?;
            if claim.interval.end_mm()
                > target.binding.revision.traffic().lane_lengths_millimetres()[target_edge.index()]
            {
                return Err(CutoverError::ConflictRevalidationFailed);
            }
            let interval = crate::DownstreamInterval::new(
                target_edge,
                claim.interval.start_mm(),
                claim.interval.end_mm(),
            )
            .ok_or(CutoverError::ConflictRevalidationFailed)?;
            mapped_source.push(interval);
        }
        mapped_source.sort_unstable();
        if mapped_source.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(CutoverError::ConflictRevalidationFailed);
        }
        let target_downstream_plan = target
            .reservation_downstream_claim_plan(target_range, target_state.length_mm)
            .map_err(|_| CutoverError::ConflictRevalidationFailed)?;
        let mut downstream = try_staging_vec(target_downstream_plan.raw_interval_capacity())?;
        target
            .derive_reservation_downstream_claims_from_plan(target_downstream_plan, &mut downstream)
            .map_err(|_| CutoverError::ConflictRevalidationFailed)?;
        if downstream != mapped_source {
            return Err(CutoverError::ConflictRevalidationFailed);
        }
        let follower_min_gap_mm = target
            .binding
            .revision
            .traffic()
            .relations()
            .vehicle_profile(target_state.profile)
            .map(|profile| profile.min_gap_mm())
            .filter(|target_gap| *target_gap == source_gap)
            .ok_or(CutoverError::ConflictRevalidationFailed)?;
        arbiter
            .write()
            .restore_reservation(
                handle,
                crate::conflict::RestoredConflictReservation {
                    follower_min_gap_mm,
                    acquired_tick: source_reservation.acquired_tick(),
                    passage_range: target_range,
                    cells: &cells,
                    downstream: &downstream,
                },
            )
            .map_err(|_| CutoverError::ConflictRevalidationFailed)?;
        restored_traversals.push((
            handle,
            ManeuverTraversalState {
                route: target_state.route,
                maneuver_occurrence_index: first_static.maneuver_index,
                phase: ManeuverTraversalPhase::Clearing {
                    admission_gate_hop: target_range.admission_gate_hop(),
                },
            },
        ));
    }

    let retention_ms = source
        .policy_gap_profiles()
        .iter()
        .chain(target.policy_gap_profiles())
        .map(|gap| gap.required_lag_ms())
        .max()
        .unwrap_or(0);
    let mut inherited_addresses: Vec<crate::ConflictPassageAddress> =
        try_staging_vec(source.conflict_read().cell_count())?;
    let mut removed_history_ready_at_ms = None;
    for (source_address, reference, live_reference) in source.conflict_read().migration_rows() {
        let target_address = mapped_conflict_address(source, target, rebinding, source_address);
        let continuous = target_address.is_some_and(|target_address| {
            conflict_passage_semantics_continuous(
                &source.binding.revision,
                &target.binding.revision,
                rebinding,
                source_address,
                target_address,
            )
        });
        if continuous {
            let target_address = target_address.expect("continuous locator has target address");
            inherited_addresses.push(target_address);
            if reference != crate::ConflictLagReference::NoHistory {
                arbiter
                    .write()
                    .restore_lag_reference(target_address, reference)
                    .map_err(|_| CutoverError::ConflictRevalidationFailed)?;
            }
            continue;
        }
        if live_reference {
            return Err(CutoverError::ConflictRevalidationFailed);
        }
        // 同一 locator 仍存在但物理语义不连续时，目标 cell 按新 cell 在
        // T_commit 建立更保守的 CutoverFloor；只有真正从目标删除的 cell
        // 才需要证明旧 lag 已超过保留期。
        if target_address.is_some() {
            continue;
        }
        let reference_time = match reference {
            crate::ConflictLagReference::NoHistory => continue,
            crate::ConflictLagReference::ActualClear(time)
            | crate::ConflictLagReference::CutoverFloor(time) => time,
        };
        if reference_time > prepare_time_ms {
            return Err(CutoverError::ConflictRevalidationFailed);
        }
        let ready_at = reference_time
            .checked_add(retention_ms)
            .ok_or(CutoverError::ConflictRevalidationFailed)?;
        removed_history_ready_at_ms = Some(
            removed_history_ready_at_ms.map_or(ready_at, |current: u64| current.max(ready_at)),
        );
    }
    inherited_addresses.sort_unstable();
    inherited_addresses.dedup();
    let mut target_addresses = try_staging_vec(arbiter.read().cell_count())?;
    target_addresses.extend(arbiter.read().addresses());
    target_addresses.retain(|address| inherited_addresses.binary_search(address).is_err());
    for address in target_addresses.iter().copied() {
        arbiter
            .write()
            .restore_lag_reference(
                address,
                crate::ConflictLagReference::CutoverFloor(prepare_time_ms),
            )
            .map_err(|_| CutoverError::ConflictRevalidationFailed)?;
    }

    (
        target.committed.conflict,
        target.derived.conflict,
        target.workspace.conflict,
    ) = arbiter.into_parts();
    target.committed.conflict_eligibility = eligibility.into_vec();
    target.normalize_conflict_eligibility();
    for (handle, traversal) in restored_traversals {
        target.committed.vehicles[handle.index() as usize]
            .state
            .as_mut()
            .ok_or(CutoverError::ConflictRevalidationFailed)?
            .maneuver_traversal = Some(traversal);
    }
    if !target.conflict_state_valid() {
        return Err(CutoverError::ConflictRevalidationFailed);
    }
    Ok(ConflictCutoverFinalizationPlan {
        floor_addresses: target_addresses,
        removed_history_ready_at_ms,
    })
}

/// 静默点只把 Prepare 已登记的新/不连续 cell 推迟到最终 `T_commit`。
/// 地址集合由目标静态根决定，循环量与本次新增/不连续 cell 数成正比。
pub(crate) fn finalize_conflict_cutover_floors(
    target: &mut TrafficWorld,
    plan: &ConflictCutoverFinalizationPlan,
    commit_time_ms: u64,
) -> Result<(), CutoverError> {
    if plan
        .removed_history_ready_at_ms
        .is_some_and(|ready_at| commit_time_ms < ready_at)
    {
        return Err(CutoverError::ConflictRevalidationFailed);
    }
    for address in plan.floor_addresses.iter().copied() {
        if !matches!(
            target.conflict_read().lag_reference(address),
            Some(crate::ConflictLagReference::CutoverFloor(_))
        ) {
            return Err(CutoverError::ConflictRevalidationFailed);
        }
        crate::conflict::ConflictWrite::new(
            &mut target.committed.conflict,
            &mut target.derived.conflict,
            &mut target.workspace.conflict,
        )
        .restore_lag_reference(
            address,
            crate::ConflictLagReference::CutoverFloor(commit_time_ms),
        )
        .map_err(|_| CutoverError::ConflictRevalidationFailed)?;
    }
    Ok(())
}

/// 从来源世界和目标静态根独立构造 digest 期望的 Conflict 语义。
/// 该路径不读取候选 arbiter/reservation 内容，用于发现候选重建偏差。
pub(crate) fn project_expected_conflict(
    source: &TrafficWorld,
    target: &TrafficWorld,
    rebinding: &CrossRevisionRebinding,
    captured: &mut crate::CapturedSnapshot,
    commit_time_ms: u64,
) -> Result<(), CutoverError> {
    let mut captured_index = 0_usize;
    for slot in &source.committed.vehicles {
        let Some(source_state) = slot.state.as_ref() else {
            continue;
        };
        let vehicle = captured
            .vehicles
            .get_mut(captured_index)
            .ok_or(CutoverError::ConflictRevalidationFailed)?;
        captured_index += 1;
        let target_state = target
            .vehicle_state(source_state.handle)
            .ok_or(CutoverError::ConflictRevalidationFailed)?;

        vehicle.conflict_eligibility = match source
            .committed
            .conflict_eligibility
            .get(source_state.handle.index() as usize)
            .copied()
            .flatten()
        {
            None => None,
            Some(eligibility) => {
                if source.conflict_reservation(source_state.handle).is_some()
                    || target_state.status != VehicleStatus::Active
                {
                    return Err(CutoverError::ConflictRevalidationFailed);
                }
                let mapped = mapped_conflict_occurrence(
                    source,
                    target,
                    rebinding,
                    source_state.route,
                    target_state.route,
                    eligibility.locator().conflict_occurrence_index(),
                )
                .and_then(|(target_occurrence_index, _)| {
                    target
                        .conflict_passage_occurrence_locator(
                            target_state.route,
                            target_occurrence_index,
                        )
                        .map(|locator| (target_occurrence_index, locator))
                });
                match mapped {
                    None => None,
                    Some((target_occurrence_index, locator)) => {
                        let migrated = crate::ConflictEligibilityState::update(
                            None,
                            locator,
                            true,
                            eligibility.first_eligible_tick(),
                        )
                        .expect("true predicate creates eligibility");
                        if !target.conflict_eligibility_authority_valid(target_state, migrated) {
                            None
                        } else {
                            let compiled = target
                                .compiled_route(target_state.route)
                                .ok_or(CutoverError::ConflictRevalidationFailed)?;
                            let gate = compiled
                                .hop_gate
                                .get(locator.admission_gate_hop() as usize)
                                .copied()
                                .flatten()
                                .ok_or(CutoverError::ConflictRevalidationFailed)?;
                            let maneuver = compiled
                                .maneuvers
                                .get(locator.maneuver_occurrence_index() as usize)
                                .ok_or(CutoverError::ConflictRevalidationFailed)?;
                            let stable = locator.stable_locator();
                            Some(crate::snapshot::CapturedConflictEligibility {
                                maneuver_occurrence_index: locator.maneuver_occurrence_index(),
                                maneuver_entry_route_edge_index: maneuver.entry_route_edge_index,
                                admission_gate: *target
                                    .binding
                                    .revision
                                    .identity()
                                    .stable_id(gate)
                                    .ok_or(CutoverError::ConflictRevalidationFailed)?
                                    .as_untyped(),
                                conflict_occurrence_index: target_occurrence_index,
                                passage: crate::snapshot::CapturedConflictPassageLocator {
                                    participant_stream: *stable
                                        .participant_stream_stable_id()
                                        .as_untyped(),
                                    conflict_zone: *stable.conflict_zone_stable_id().as_untyped(),
                                },
                                first_eligible_tick: eligibility.first_eligible_tick(),
                            })
                        }
                    }
                }
            }
        };

        let Some(source_reservation) = source.conflict_reservation(source_state.handle) else {
            vehicle.conflict_reservation = None;
            continue;
        };
        let source_range = source_reservation.passage_range();
        let end = source_range
            .first_conflict_occurrence_index()
            .checked_add(source_range.passage_count())
            .ok_or(CutoverError::ConflictRevalidationFailed)?;
        let mut target_occurrences = try_staging_vec(source_range.passage_count() as usize)?;
        for source_occurrence in source_range.first_conflict_occurrence_index()..end {
            target_occurrences.push(
                mapped_conflict_occurrence(
                    source,
                    target,
                    rebinding,
                    source_state.route,
                    target_state.route,
                    source_occurrence,
                )
                .ok_or(CutoverError::ConflictRevalidationFailed)?,
            );
        }
        target_occurrences.sort_unstable_by_key(|(index, _)| *index);
        let target_compiled = target
            .compiled_route(target_state.route)
            .ok_or(CutoverError::ConflictRevalidationFailed)?;
        let first = target_occurrences
            .first()
            .map(|(index, _)| *index)
            .ok_or(CutoverError::ConflictRevalidationFailed)?;
        let first_static = target_compiled
            .conflicts
            .get(first as usize)
            .ok_or(CutoverError::ConflictRevalidationFailed)?;
        let maneuver = target_compiled
            .maneuvers
            .get(first_static.maneuver_index as usize)
            .ok_or(CutoverError::ConflictRevalidationFailed)?;
        let gate = target_compiled
            .hop_gate
            .get(first_static.admission_hop as usize)
            .copied()
            .flatten()
            .ok_or(CutoverError::ConflictRevalidationFailed)?;
        let gate_stable = *target
            .binding
            .revision
            .identity()
            .stable_id(gate)
            .ok_or(CutoverError::ConflictRevalidationFailed)?
            .as_untyped();
        let path_stable = *target
            .binding
            .revision
            .identity()
            .stable_id(maneuver.path)
            .ok_or(CutoverError::ConflictRevalidationFailed)?
            .as_untyped();
        let mut passages = try_staging_vec(target_occurrences.len())?;
        for (index, _) in target_occurrences {
            let occurrence = target_compiled
                .conflicts
                .get(index as usize)
                .ok_or(CutoverError::ConflictRevalidationFailed)?;
            let locator = target
                .conflict_passage_occurrence_locator(target_state.route, index)
                .ok_or(CutoverError::ConflictRevalidationFailed)?
                .stable_locator();
            passages.push(crate::snapshot::CapturedConflictPassage {
                conflict_occurrence_index: index,
                passage: crate::snapshot::CapturedConflictPassageLocator {
                    participant_stream: *locator.participant_stream_stable_id().as_untyped(),
                    conflict_zone: *locator.conflict_zone_stable_id().as_untyped(),
                },
                entry_route_edge_index: occurrence.entry.route_edge_index,
                entry_progress_mm: occurrence.entry.progress_mm,
                clearance_route_edge_index: occurrence.clearance.route_edge_index,
                clearance_progress_mm: occurrence.clearance.progress_mm,
            });
        }
        let existing = vehicle
            .conflict_reservation
            .as_mut()
            .ok_or(CutoverError::ConflictRevalidationFailed)?;
        let target_range = crate::ConflictPassageRange::new(
            target_state.route,
            first_static.maneuver_index,
            first_static.admission_hop,
            first,
            u32::try_from(passages.len()).map_err(|_| CutoverError::ConflictRevalidationFailed)?,
        )
        .ok_or(CutoverError::ConflictRevalidationFailed)?;
        let downstream_plan = target
            .reservation_downstream_claim_plan(target_range, target_state.length_mm)
            .map_err(|_| CutoverError::ConflictRevalidationFailed)?;
        let mut physical = try_staging_vec(downstream_plan.raw_interval_capacity())?;
        target
            .derive_reservation_downstream_claims_from_plan(downstream_plan, &mut physical)
            .map_err(|_| CutoverError::ConflictRevalidationFailed)?;
        let mut downstream_intervals = try_staging_vec(physical.len())?;
        for interval in physical {
            downstream_intervals.push(crate::snapshot::CapturedConflictDownstreamInterval {
                lane_edge: *target
                    .binding
                    .revision
                    .identity()
                    .stable_id(interval.edge())
                    .ok_or(CutoverError::ConflictRevalidationFailed)?
                    .as_untyped(),
                start_mm: interval.start_mm(),
                end_mm: interval.end_mm(),
            });
        }
        downstream_intervals.sort_unstable_by_key(|interval| {
            (interval.lane_edge, interval.start_mm, interval.end_mm)
        });
        existing.maneuver_occurrence_index = first_static.maneuver_index;
        existing.maneuver_entry_route_edge_index = maneuver.entry_route_edge_index;
        existing.admission_gate = gate_stable;
        existing.passages = passages;
        existing.downstream_intervals = downstream_intervals;
        let traversal = vehicle
            .maneuver_traversal
            .as_mut()
            .ok_or(CutoverError::ConflictRevalidationFailed)?;
        traversal.maneuver_occurrence_index = first_static.maneuver_index;
        traversal.maneuver_path = path_stable;
        traversal.phase = crate::snapshot::CapturedManeuverTraversalPhase::Clearing;
        traversal.phase_gate = gate_stable;
    }
    if captured_index != captured.vehicles.len() {
        return Err(CutoverError::ConflictRevalidationFailed);
    }

    let mut inherited: Vec<(crate::ConflictPassageAddress, crate::ConflictPassageAddress)> =
        try_staging_vec(source.conflict_read().cell_count())?;
    for source_address in source.conflict_read().addresses() {
        if let Some(target_address) =
            mapped_conflict_address(source, target, rebinding, source_address)
            && conflict_passage_semantics_continuous(
                &source.binding.revision,
                &target.binding.revision,
                rebinding,
                source_address,
                target_address,
            )
        {
            inherited.push((target_address, source_address));
        }
    }
    inherited.sort_unstable_by_key(|(target, _)| *target);
    let mut lag_states = try_staging_vec(target.conflict_read().cell_count())?;
    for target_address in target.conflict_read().addresses() {
        let reference = inherited
            .binary_search_by_key(&target_address, |(target, _)| *target)
            .ok()
            .and_then(|index| source.conflict_read().lag_reference(inherited[index].1))
            .unwrap_or(crate::ConflictLagReference::CutoverFloor(commit_time_ms));
        if reference == crate::ConflictLagReference::NoHistory {
            continue;
        }
        let locator = target
            .conflict_passage_locator(target_address)
            .ok_or(CutoverError::ConflictRevalidationFailed)?;
        lag_states.push(crate::snapshot::CapturedConflictLagState {
            passage: crate::snapshot::CapturedConflictPassageLocator {
                participant_stream: *locator.participant_stream_stable_id().as_untyped(),
                conflict_zone: *locator.conflict_zone_stable_id().as_untyped(),
            },
            reference,
        });
    }
    lag_states.sort_unstable_by_key(|state| {
        (
            state.passage.participant_stream,
            state.passage.conflict_zone,
        )
    });
    captured.conflict_lag_states = lag_states;
    Ok(())
}

/// 重绑即重验证：对候选内每个 live 车辆按其生命周期状态复核 target
/// 不变量（切换合同 §3 原则 2，等价重执行 spawn 检查）。
pub(crate) fn revalidate_migrated_vehicles(
    candidate: &mut TrafficWorld,
) -> Result<(), CutoverError> {
    if !candidate.waiting_state_valid() || !candidate.waiting_snapshot_storage_valid() {
        return Err(CutoverError::WaitingRevalidationFailed);
    }
    if !candidate.conflict_state_valid() {
        return Err(CutoverError::ConflictRevalidationFailed);
    }
    candidate
        .prepare_waiting_dependencies(false)
        .map_err(|error| match error {
            crate::StepError::ConflictScratchAllocFailed => CutoverError::StagingAllocFailed,
            _ => CutoverError::WaitingRevalidationFailed,
        })?;
    for handle in candidate.committed.live_order.iter().copied() {
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
    let traffic = candidate.binding.revision.traffic();
    let lengths = traffic.lane_lengths_millimetres();
    let speed_limits = traffic.lane_speed_limits_millimetres_per_second();
    let state = candidate
        .vehicle_state(handle)
        .ok_or(CutoverError::VehicleRevalidationFailed {
            vehicle: handle.index(),
        })?;
    if !candidate.restored_waiting_authority_valid(*state) {
        return Err(CutoverError::WaitingRevalidationFailed);
    }
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
        for other in candidate.committed.live_order.iter().copied() {
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
            Err(crate::tables::ConflictCapabilityError::AuthorityRequired)
                if candidate.conflict_reservation(handle).is_some()
                    || candidate
                        .committed
                        .conflict_eligibility
                        .get(handle.index() as usize)
                        .is_some_and(Option::is_some) => {}
            Err(crate::tables::ConflictCapabilityError::AuthorityRequired) => {
                return Err(CutoverError::VehicleRevalidationFailed {
                    vehicle: handle.index(),
                });
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
        SyntheticModuleBuilder, VehicleProfileInput, emit_portable_candidate, road_editing as lfre,
    };
    use laneflow_format::{
        FormatLimits, check_canonical_network_input, check_post_emission_bundle,
        preflight_object_values,
    };
    use laneflow_static_contract::{
        EntityKind, ExactByteLength, LaneEdgeId, ParticipantStreamOrdinal, PortableObjectKind,
        RightOfWayPolicySetId, SEMANTIC_DIFF_FORMAT_VERSION, Sha256Digest, StableId128,
        VehicleProfileOrdinal,
    };
    use laneflow_static_network::{
        CanonicalNetworkOrigin, SharedNetworkBuildLimits, SharedNetworkBuildOptions,
        SpatialBuildOption, build_shared_network_revision,
    };
    use sha2::Digest as _;

    use super::*;
    use crate::cutover::tests::transaction_tests::revision as conflict_revision;
    use crate::snapshot_restore::tests::{
        install_conflict_reservation, world_with_conflict_eligibility,
        world_with_conflict_reservation,
    };
    use crate::{
        CUTOVER_DESCRIPTOR_FORMAT_VERSION, CutoverPreflightLimits, CutoverTransactionLimits,
        LfcaOriginBinding, MigrationPolicyKind, NetworkRevisionCutoverDescriptor,
        ParkedVehicleSpawnInput, ParkingTarget, PolicyPin, PoseSource, PublishedLfcaReference,
        ReserveParkingTarget, RouteRegisterInput, SemanticDiffOriginBinding, TickInput,
        VehicleSpawnInput, VehicleStatus, VirtualEntryAnchorSelector, WorldBinding, WorldConfig,
        WorldGeneration, WorldPolicySelection,
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

    fn conflict_cutover_test_line(
        start: (f64, f64),
        end: (f64, f64),
    ) -> lfre::RoadEditingCurveProgram {
        lfre::RoadEditingCurveProgram::try_new(
            lfre::RoadEditingPoint3::try_new(start.0, 0.0, start.1).expect("curve start"),
            vec![lfre::RoadEditingCurveSegment::line(
                lfre::RoadEditingPoint3::try_new(end.0, 0.0, end.1).expect("curve end"),
            )],
        )
        .expect("line curve")
    }

    fn conflict_scale_loop() -> lfre::RoadEditingCurveProgram {
        let point = |x, z| lfre::RoadEditingPoint3::try_new(x, 0.0, z).expect("scale point");
        // Keep each closed approach below the 10 km static-contract ceiling while
        // providing enough distinct route length for the 100k population fixture.
        const RADIUS: f64 = 1_500.0;
        const K: f64 = RADIUS * 0.552_284_749_830_793_6;
        const X: f64 = -13.0;
        lfre::RoadEditingCurveProgram::try_new(
            point(X, 0.0),
            vec![
                lfre::RoadEditingCurveSegment::cubic_bezier(
                    point(X + K, 0.0),
                    point(X + RADIUS, RADIUS - K),
                    point(X + RADIUS, RADIUS),
                ),
                lfre::RoadEditingCurveSegment::cubic_bezier(
                    point(X + RADIUS, RADIUS + K),
                    point(X + K, 2.0 * RADIUS),
                    point(X, 2.0 * RADIUS),
                ),
                lfre::RoadEditingCurveSegment::cubic_bezier(
                    point(X - K, 2.0 * RADIUS),
                    point(X - RADIUS, RADIUS + K),
                    point(X - RADIUS, RADIUS),
                ),
                lfre::RoadEditingCurveSegment::cubic_bezier(
                    point(X - RADIUS, RADIUS - K),
                    point(X - K, 0.0),
                    point(X, 0.0),
                ),
            ],
        )
        .expect("scale loop")
    }

    fn add_conflict_cutover_test_approach(
        module: &mut lfre::RoadEditingSourceModuleBuilder<'_>,
        edge_key: &str,
        geometry: lfre::RoadEditingCurveProgram,
        successors: Vec<lfre::LaneEdgeReference>,
    ) {
        let alignment_key = format!("{edge_key}-alignment");
        let corridor_key = format!("{edge_key}-corridor");
        let corridor =
            lfre::RoadCorridorReference::local(&corridor_key).expect("corridor reference");
        let section =
            lfre::RoadSectionReference::owner_scoped(vec![corridor_key.clone()], "section")
                .expect("section reference");
        let lane = lfre::AuthoringLaneReference::owner_scoped(
            vec![corridor_key.clone(), "section".into()],
            "lane",
        )
        .expect("authoring lane reference");
        let edge = lfre::LaneEdgeReference::local(edge_key).expect("approach edge reference");
        module
            .add_alignment(
                lfre::RoadAlignmentInput::try_new(
                    &alignment_key,
                    lfre::CanonicalFrameReference::local("frame").expect("frame reference"),
                    geometry,
                )
                .expect("road alignment"),
            )
            .expect("add road alignment")
            .add_declaration(lfre::RoadEditingDeclaration::RoadCorridor(
                lfre::RoadCorridorInput::try_new(
                    &corridor_key,
                    lfre::RoadAlignmentReference::try_new(&alignment_key)
                        .expect("alignment reference"),
                    0.0,
                    lfre::RoadEditingStationEnd::AlignmentEnd,
                    section.clone(),
                    lane.clone(),
                    vec![lfre::RoadEditingCorridorElement::RoadSection(
                        section.clone(),
                    )],
                )
                .expect("road corridor"),
            ))
            .expect("add road corridor")
            .add_declaration(lfre::RoadEditingDeclaration::RoadSection(
                lfre::RoadSectionInput::try_new("section", "motorLane", vec![lane], corridor)
                    .expect("road section"),
            ))
            .expect("add road section")
            .add_declaration(lfre::RoadEditingDeclaration::AuthoringLane(
                lfre::AuthoringLaneInput::try_new(
                    "lane",
                    edge,
                    lfre::RoadEditingLaneDirection::Forward,
                    lfre::LinearWidthProfile::try_new(3.5, 3.5).expect("lane width"),
                    None,
                    section,
                )
                .expect("authoring lane"),
            ))
            .expect("add authoring lane")
            .add_declaration(lfre::RoadEditingDeclaration::LaneEdge(
                lfre::LaneEdgeInput::try_new(edge_key, 13.0, successors, None)
                    .expect("approach lane edge"),
            ))
            .expect("add approach lane edge");
    }

    fn conflict_cutover_test_module(
        insert_preceding_passage: bool,
        change_stable_passage_exit: bool,
        long_approaches: bool,
        yielding: bool,
    ) -> lfre::RoadEditingSourceModule {
        let limits = if long_approaches {
            CompileLimits::single_network_1m_v2()
        } else {
            CompileLimits::p100_initial_v2()
        };
        let header = lfre::RoadEditingModuleHeader::try_new(
            "city/runtime-live-conflict-cutover",
            "runtime-live-conflict-cutover.lfre",
            Vec::new(),
            lfre::RoadEditingProvenance::direct("runtime live Conflict cutover fixture")
                .expect("provenance"),
        )
        .expect("Road Editing header");
        let mut module = lfre::RoadEditingSourceModuleBuilder::new(
            header,
            laneflow_compiler::GeometryAccuracyProfile::Balanced5Cm,
            laneflow_compiler::GeometryDirectionProfile::Balanced2Deg,
            &limits,
        )
        .expect("Road Editing builder");
        let junction = lfre::JunctionReference::local("crossing").expect("junction reference");
        let frame = lfre::CanonicalFrameReference::local("frame").expect("frame reference");
        let path = lfre::ManeuverPathReference::owner_scoped(
            vec!["crossing".into(), "through".into()],
            "path",
        )
        .expect("path reference");
        let gate = lfre::ManeuverGateReference::owner_scoped(
            vec!["crossing".into(), "through".into(), "path".into()],
            "admission",
        )
        .expect("Gate reference");
        let other_path = lfre::ManeuverPathReference::owner_scoped(
            vec!["crossing".into(), "other-through".into()],
            "other-path",
        )
        .expect("other path reference");
        let other_gate = lfre::ManeuverGateReference::owner_scoped(
            vec![
                "crossing".into(),
                "other-through".into(),
                "other-path".into(),
            ],
            "other-admission",
        )
        .expect("other Gate reference");
        let stable_zone =
            lfre::ConflictZoneReference::owner_scoped(vec!["crossing".into()], "z-stable")
                .expect("stable zone reference");
        let inserted_zone =
            lfre::ConflictZoneReference::owner_scoped(vec!["crossing".into()], "a-inserted")
                .expect("inserted zone reference");

        module
            .add_declaration(lfre::RoadEditingDeclaration::CanonicalFrame(
                lfre::CanonicalFrameInput::try_new("frame").expect("canonical frame"),
            ))
            .expect("add frame");
        if long_approaches {
            for index in 0..70 {
                let edge = format!("scale-stem-{index:02}");
                let successor = if index == 69 {
                    "entry".to_owned()
                } else {
                    format!("scale-stem-{:02}", index + 1)
                };
                add_conflict_cutover_test_approach(
                    &mut module,
                    &edge,
                    conflict_scale_loop(),
                    vec![lfre::LaneEdgeReference::local(successor).expect("scale successor")],
                );
            }
        }
        add_conflict_cutover_test_approach(
            &mut module,
            "entry",
            conflict_cutover_test_line((-13.0, 0.0), (0.0, 0.0)),
            Vec::new(),
        );
        add_conflict_cutover_test_approach(
            &mut module,
            "exit",
            conflict_cutover_test_line((13.0, 0.0), (26.0, 0.0)),
            Vec::new(),
        );
        add_conflict_cutover_test_approach(
            &mut module,
            "other-entry",
            conflict_cutover_test_line((0.0, -13.0), (0.0, 0.0)),
            Vec::new(),
        );
        add_conflict_cutover_test_approach(
            &mut module,
            "other-exit",
            conflict_cutover_test_line((0.0, 13.0), (0.0, 26.0)),
            Vec::new(),
        );
        module
            .add_declaration(lfre::RoadEditingDeclaration::LaneEdge(
                lfre::LaneEdgeInput::try_new(
                    "internal",
                    13.0,
                    Vec::new(),
                    Some(conflict_cutover_test_line((0.0, 0.0), (13.0, 0.0))),
                )
                .expect("internal lane edge"),
            ))
            .expect("add internal lane edge")
            .add_declaration(lfre::RoadEditingDeclaration::LaneEdge(
                lfre::LaneEdgeInput::try_new(
                    "other-internal",
                    13.0,
                    Vec::new(),
                    Some(conflict_cutover_test_line((0.0, 0.0), (0.0, 13.0))),
                )
                .expect("other internal lane edge"),
            ))
            .expect("add other internal lane edge");
        module
            .add_declaration(lfre::RoadEditingDeclaration::Junction(
                lfre::JunctionInput::try_new(
                    "crossing",
                    ["entry", "exit", "other-entry", "other-exit"]
                        .into_iter()
                        .map(|edge| lfre::LaneEdgeReference::local(edge).expect("approach"))
                        .collect(),
                    ["internal", "other-internal"]
                        .into_iter()
                        .map(|edge| lfre::LaneEdgeReference::local(edge).expect("internal"))
                        .collect(),
                )
                .expect("junction"),
            ))
            .expect("add junction")
            .add_declaration(lfre::RoadEditingDeclaration::Movement(
                lfre::MovementInput::try_new("through", junction.clone(), "entry", "exit")
                    .expect("movement"),
            ))
            .expect("add movement")
            .add_declaration(lfre::RoadEditingDeclaration::ManeuverPath(
                lfre::ManeuverPathInput::try_new(
                    "path",
                    lfre::MovementReference::owner_scoped(vec!["crossing".into()], "through")
                        .expect("movement reference"),
                    lfre::LaneEdgeReference::local("entry").expect("entry"),
                    vec![lfre::LaneEdgeReference::local("internal").expect("internal")],
                    lfre::LaneEdgeReference::local("exit").expect("exit"),
                )
                .expect("maneuver path"),
            ))
            .expect("add maneuver path")
            .add_declaration(lfre::RoadEditingDeclaration::StopLine(
                lfre::StopLineInput::try_new(
                    "stop",
                    lfre::LaneEdgeReference::local("entry").expect("stop edge"),
                )
                .expect("stop line"),
            ))
            .expect("add stop line")
            .add_declaration(lfre::RoadEditingDeclaration::ManeuverGate(
                lfre::ManeuverGateInput::try_new(
                    "admission",
                    path.clone(),
                    0,
                    lfre::StopLineReference::local("stop").expect("stop line reference"),
                    lfre::RoadEditingSignalControl::None,
                )
                .expect("maneuver Gate"),
            ))
            .expect("add maneuver Gate")
            .add_declaration(lfre::RoadEditingDeclaration::Movement(
                lfre::MovementInput::try_new(
                    "other-through",
                    junction.clone(),
                    "other-entry",
                    "other-exit",
                )
                .expect("other movement"),
            ))
            .expect("add other movement")
            .add_declaration(lfre::RoadEditingDeclaration::ManeuverPath(
                lfre::ManeuverPathInput::try_new(
                    "other-path",
                    lfre::MovementReference::owner_scoped(vec!["crossing".into()], "other-through")
                        .expect("other movement reference"),
                    lfre::LaneEdgeReference::local("other-entry").expect("other entry"),
                    vec![lfre::LaneEdgeReference::local("other-internal").expect("other internal")],
                    lfre::LaneEdgeReference::local("other-exit").expect("other exit"),
                )
                .expect("other maneuver path"),
            ))
            .expect("add other maneuver path")
            .add_declaration(lfre::RoadEditingDeclaration::StopLine(
                lfre::StopLineInput::try_new(
                    "other-stop",
                    lfre::LaneEdgeReference::local("other-entry").expect("other stop edge"),
                )
                .expect("other stop line"),
            ))
            .expect("add other stop line")
            .add_declaration(lfre::RoadEditingDeclaration::ManeuverGate(
                lfre::ManeuverGateInput::try_new(
                    "other-admission",
                    other_path.clone(),
                    0,
                    lfre::StopLineReference::local("other-stop")
                        .expect("other stop line reference"),
                    lfre::RoadEditingSignalControl::None,
                )
                .expect("other maneuver Gate"),
            ))
            .expect("add other maneuver Gate")
            .add_declaration(lfre::RoadEditingDeclaration::ConflictZone(
                lfre::ConflictZoneInput::try_new("z-stable", junction.clone())
                    .expect("stable zone"),
            ))
            .expect("add stable zone");
        if insert_preceding_passage {
            module
                .add_declaration(lfre::RoadEditingDeclaration::ConflictZone(
                    lfre::ConflictZoneInput::try_new("a-inserted", junction.clone())
                        .expect("inserted zone"),
                ))
                .expect("add inserted zone");
        }
        let stable_passage = lfre::ConflictPassageInput::new(
            stable_zone.clone(),
            lfre::PathAnchorInput::interior(1, 3.0).expect("stable entry"),
            lfre::PathAnchorInput::interior(1, if change_stable_passage_exit { 9.0 } else { 8.0 })
                .expect("stable exit"),
        );
        module
            .add_declaration(lfre::RoadEditingDeclaration::ParticipantStream(
                lfre::ParticipantStreamInput::try_new(
                    "stream",
                    junction.clone(),
                    path.clone(),
                    vec![stable_passage],
                )
                .expect("participant stream"),
            ))
            .expect("add participant stream");
        let other_stable_passage = lfre::ConflictPassageInput::new(
            stable_zone.clone(),
            lfre::PathAnchorInput::interior(1, 3.5).expect("other stable entry"),
            lfre::PathAnchorInput::interior(1, 7.5).expect("other stable exit"),
        );
        let other_passages = if insert_preceding_passage {
            vec![
                lfre::ConflictPassageInput::new(
                    inserted_zone.clone(),
                    lfre::PathAnchorInput::interior(1, 1.2).expect("other inserted entry"),
                    lfre::PathAnchorInput::interior(1, 1.8).expect("other inserted exit"),
                ),
                other_stable_passage,
            ]
        } else {
            vec![other_stable_passage]
        };
        module
            .add_declaration(lfre::RoadEditingDeclaration::ParticipantStream(
                lfre::ParticipantStreamInput::try_new(
                    "other-stream",
                    junction.clone(),
                    other_path.clone(),
                    other_passages,
                )
                .expect("other participant stream"),
            ))
            .expect("add other participant stream");
        if insert_preceding_passage {
            module
                .add_declaration(lfre::RoadEditingDeclaration::ParticipantStream(
                    lfre::ParticipantStreamInput::try_new(
                        "inserted-stream",
                        junction.clone(),
                        other_path.clone(),
                        vec![lfre::ConflictPassageInput::new(
                            inserted_zone.clone(),
                            lfre::PathAnchorInput::interior(1, 1.3).expect("inserted peer entry"),
                            lfre::PathAnchorInput::interior(1, 1.7).expect("inserted peer exit"),
                        )],
                    )
                    .expect("inserted peer stream"),
                ))
                .expect("add inserted peer stream");
        }
        for (zone, min_x, max_x) in [(stable_zone, -1.0, 1.0), (inserted_zone, -3.0, -2.0)]
            .into_iter()
            .take(if insert_preceding_passage { 2 } else { 1 })
        {
            module
                .add_conflict_zone_region(
                    lfre::ConflictZoneRegionInput::try_new(
                        zone,
                        frame.clone(),
                        -1.0,
                        1.0,
                        [(min_x, -1.0), (max_x, -1.0), (max_x, 1.0), (min_x, 1.0)]
                            .into_iter()
                            .map(|(x, z)| {
                                lfre::RoadEditingPoint2::try_new(x, z).expect("region point")
                            })
                            .collect(),
                    )
                    .expect("conflict zone region"),
                )
                .expect("add conflict zone region");
        }
        let stream_rule = |stream| {
            let yield_to = (yielding && stream == "stream")
                .then(|| {
                    lfre::ParticipantStreamReference::owner_scoped(
                        vec!["crossing".into()],
                        "other-stream",
                    )
                    .expect("yield target")
                })
                .into_iter()
                .collect();
            lfre::PolicyStreamRuleInput::try_new(
                stream,
                lfre::ParticipantStreamReference::owner_scoped(vec!["crossing".into()], stream)
                    .expect("stream reference"),
                None,
                i32::from(stream == "other-stream"),
                yield_to,
                (yielding && stream == "stream").then(|| "calibration-gap".to_owned()),
                vec![],
            )
            .expect("stream policy")
        };
        let mut policy_streams = vec![stream_rule("stream"), stream_rule("other-stream")];
        if insert_preceding_passage {
            policy_streams.push(stream_rule("inserted-stream"));
        }
        let participant =
            lfre::ParticipantClassReference::local("road-user").expect("participant class");
        module
            .add_declaration(lfre::RoadEditingDeclaration::ParticipantClass(
                lfre::ParticipantClassInput::try_new("road-user").expect("participant class"),
            ))
            .expect("add participant class")
            .add_declaration(lfre::RoadEditingDeclaration::VehicleProfile(
                lfre::VehicleProfileInput::try_new(
                    "car",
                    participant,
                    lfre::IidmVehicleProfileInput::try_new(4.5, 13.0, 2.0, 1.5, 1.5, 2.0, 4.0)
                        .expect("vehicle profile"),
                )
                .expect("vehicle profile"),
            ))
            .expect("add vehicle profile")
            .add_declaration(lfre::RoadEditingDeclaration::RightOfWayPolicySet(
                lfre::RightOfWayPolicySetInput::try_new(
                    "policy",
                    laneflow_compiler::RegulationIdentity::try_new("engineering", "fixture-1")
                        .expect("regulation")
                        .with_source("repository:runtime-live-conflict-cutover")
                        .expect("regulation source"),
                    vec![],
                    yielding
                        .then(|| {
                            lfre::PolicyGapProfileInput::try_new(
                                "calibration-gap",
                                "fixture-1",
                                500,
                                500,
                                100,
                            )
                            .expect("calibration gap")
                        })
                        .into_iter()
                        .collect(),
                    policy_streams,
                    vec![
                        lfre::PolicyGateRuleInput::try_new(
                            "admission",
                            gate,
                            None,
                            laneflow_compiler::GateInterpretation::Uncontrolled,
                            laneflow_compiler::GateProhibition::None,
                            vec![],
                        )
                        .expect("Gate policy"),
                        lfre::PolicyGateRuleInput::try_new(
                            "other-admission",
                            other_gate,
                            None,
                            laneflow_compiler::GateInterpretation::Uncontrolled,
                            laneflow_compiler::GateProhibition::None,
                            vec![],
                        )
                        .expect("other Gate policy"),
                    ],
                )
                .expect("right-of-way policy"),
            ))
            .expect("add right-of-way policy");
        module.finish().expect("Road Editing module")
    }

    pub(crate) fn conflict_scale_revision() -> Arc<SharedNetworkRevision> {
        let limits = CompileLimits::single_network_1m_v2();
        let source = lfre::RoadEditingSourceWriter::new(&limits)
            .write(conflict_cutover_test_module(false, false, true, true))
            .expect("Conflict scale Road Editing source");
        let input = lfre::RoadEditingModuleInput::try_new(
            "runtime-live-conflict-cutover.lfre",
            source.as_bytes(),
            None,
        )
        .expect("Conflict scale Road Editing input");
        let mut unit = CompilationUnitBuilder::new(limits);
        unit.add_road_editing_module(input)
            .expect("Conflict scale module admission");
        let output = Compiler::new()
            .compile(unit.build().expect("Conflict scale compilation unit"))
            .unwrap_or_else(|bundle| {
                panic!(
                    "Conflict scale compile diagnostics: {:?}",
                    bundle
                        .diagnostics()
                        .iter()
                        .map(|diagnostic| (diagnostic.code(), diagnostic.payload()))
                        .collect::<Vec<_>>()
                )
            });
        let provenance = PortableEmissionProvenance::try_new("runtime-conflict-scale-v1")
            .expect("Conflict scale provenance");
        let candidate = emit_portable_candidate(
            &output,
            &provenance,
            FormatLimits::HARD,
            PortableDiffBase::Genesis,
        )
        .expect("Conflict scale portable candidate");
        let checked = check_post_emission_bundle(
            candidate.canonical_artifact().bytes(),
            candidate.source_map().bytes(),
            candidate.semantic_diff().bytes(),
            candidate.expected_semantic_diff_base(),
            FormatLimits::HARD,
        )
        .expect("Conflict scale checked bundle");
        build_shared_network_revision(
            checked.canonical_network_input(),
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::Omit,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .expect("Conflict scale shared revision")
    }

    pub(crate) fn conflict_scale_world(
        revision: Arc<SharedNetworkRevision>,
        vehicle_count: u32,
    ) -> TrafficWorld {
        const SPACING_MM: u64 = 6_500;

        let stream = revision
            .conflict()
            .participant_stream(ParticipantStreamOrdinal::from_raw(0))
            .expect("Conflict scale stream");
        let cycle = revision
            .traffic()
            .maneuvers()
            .maneuver_path(stream.maneuver_path())
            .expect("Conflict scale path")
            .edges()
            .to_vec();
        assert_eq!(cycle.len(), 3);
        let limits = CompileLimits::single_network_1m_v2();
        let mut route_edges = (0..70)
            .map(|index| {
                let stable = laneflow_compiler::derive_canonical_stable_id_v1(
                    EntityKind::LaneEdge,
                    "city/runtime-live-conflict-cutover",
                    &format!("scale-stem-{index:02}"),
                    &limits,
                )
                .expect("Conflict scale stem identity");
                revision
                    .identity()
                    .ordinal(LaneEdgeId::from_untyped(stable))
                    .expect("Conflict scale stem ordinal")
            })
            .collect::<Vec<_>>();
        route_edges.extend_from_slice(&cycle);
        let span_mm = u64::from(vehicle_count.saturating_sub(1))
            .checked_mul(SPACING_MM)
            .and_then(|span| span.checked_add(1))
            .expect("Conflict scale population span");
        let route_lengths = route_edges
            .iter()
            .map(|edge| u64::from(revision.traffic().lane_lengths_millimetres()[edge.index()]))
            .collect::<Vec<_>>();
        let mut route_ends = Vec::with_capacity(route_lengths.len());
        let mut route_length_mm = 0_u64;
        for length in &route_lengths {
            route_length_mm = route_length_mm.checked_add(*length).expect("route length");
            route_ends.push(route_length_mm);
        }
        let entry_route_index = 70_usize;
        let entry_end_mm = route_ends[entry_route_index];
        assert!(entry_end_mm > span_mm);

        let origin = *revision.canonical_origin();
        let mut world = TrafficWorld::install(
            Arc::clone(&revision),
            WorldConfig::new(vehicle_count, 1, route_edges.len() as u64, 1, 1, 4),
            CommittedNetworkSource::Published {
                reference: PublishedLfcaReference::new(
                    "fixture://conflict-scale",
                    origin.canonical_artifact_digest(),
                    origin.canonical_artifact_byte_length(),
                    origin.network_revision(),
                )
                .expect("Conflict scale source"),
            },
            u64::from(vehicle_count),
            WorldPolicySelection::Pinned(PolicyPin {
                policy: RightOfWayPolicySetId::from_untyped(
                    laneflow_compiler::derive_canonical_stable_id_v1(
                        EntityKind::RightOfWayPolicySet,
                        "city/runtime-live-conflict-cutover",
                        "policy",
                        &CompileLimits::single_network_1m_v2(),
                    )
                    .expect("Conflict scale policy identity"),
                ),
            }),
        )
        .expect("Conflict scale world install");
        let route = world
            .register_route(RouteRegisterInput::new(route_edges))
            .expect("Conflict scale route");
        let profile = VehicleProfileOrdinal::from_raw(0);
        let profile_view = world
            .traffic()
            .relations()
            .vehicle_profile(profile)
            .expect("Conflict scale vehicle profile");
        let profile_class = profile_view.class();
        let profile_length_mm = profile_view.length_mm();
        let frontmost_mm = entry_end_mm - 1;
        for update_sequence in 0..vehicle_count {
            let absolute_mm = frontmost_mm
                .checked_sub(u64::from(update_sequence) * SPACING_MM)
                .expect("Conflict scale route has room");
            let route_edge_index = route_ends.partition_point(|end| *end <= absolute_mm);
            let edge_start_mm = route_edge_index
                .checked_sub(1)
                .map_or(0, |previous| route_ends[previous]);
            let progress_mm =
                u32::try_from(absolute_mm - edge_start_mm).expect("Conflict scale edge progress");
            let route_edge_index =
                u32::try_from(route_edge_index).expect("Conflict scale route occurrence");
            let handle = VehicleHandle::new(update_sequence, 0);
            world.committed.vehicles.push(VehicleSlot {
                generation: 0,
                state: Some(VehicleState {
                    handle,
                    profile,
                    class: profile_class,
                    route,
                    route_edge_index,
                    progress_mm,
                    carry_um: 999,
                    speed_mm_s: 10_000,
                    length_mm: profile_length_mm,
                    status: VehicleStatus::Active,
                    maneuver_traversal: None,
                    waiting_membership: None,
                }),
            });
            world.committed.live_order.push(handle);
            world.derived.active_order.push(handle);
        }
        world.committed.routes[route.index() as usize].live_vehicles = vehicle_count;
        world
            .rebuild_occupancy_index()
            .expect("Conflict scale occupancy");
        world
    }

    fn compile_conflict_cutover_test_pair(
        target_insert_preceding_passage: bool,
        target_change_stable_passage_exit: bool,
    ) -> (
        Arc<SharedNetworkRevision>,
        Arc<SharedNetworkRevision>,
        Vec<u8>,
        SemanticDiffOriginBinding,
    ) {
        let compile = |module| {
            let limits = CompileLimits::p100_initial_v2();
            let source = lfre::RoadEditingSourceWriter::new(&limits)
                .write(module)
                .expect("Road Editing source");
            let input = lfre::RoadEditingModuleInput::try_new(
                "runtime-live-conflict-cutover.lfre",
                source.as_bytes(),
                None,
            )
            .expect("Road Editing module input");
            let mut unit = CompilationUnitBuilder::new(limits);
            unit.add_road_editing_module(input)
                .expect("Road Editing admission");
            Compiler::new()
                .compile(unit.build().expect("compilation unit"))
                .unwrap_or_else(|bundle| {
                    panic!(
                        "compile diagnostics: {:?}",
                        bundle
                            .diagnostics()
                            .iter()
                            .map(|diagnostic| (diagnostic.code(), diagnostic.payload()))
                            .collect::<Vec<_>>()
                    )
                })
        };
        let base_output = compile(conflict_cutover_test_module(false, false, false, false));
        let target_output = compile(conflict_cutover_test_module(
            target_insert_preceding_passage,
            target_change_stable_passage_exit,
            false,
            false,
        ));
        let provenance = PortableEmissionProvenance::try_new("runtime-live-conflict-cutover-v1")
            .expect("portable provenance");
        let base_candidate = emit_portable_candidate(
            &base_output,
            &provenance,
            FormatLimits::HARD,
            PortableDiffBase::Genesis,
        )
        .expect("base portable candidate");
        let base_values = preflight_object_values(
            base_candidate.canonical_artifact().bytes(),
            PortableObjectKind::CanonicalArtifact,
            FormatLimits::HARD,
        )
        .expect("base artifact values");
        let target_candidate = emit_portable_candidate(
            &target_output,
            &provenance,
            FormatLimits::HARD,
            PortableDiffBase::Artifact(base_values),
        )
        .expect("target portable candidate");
        let base_checked = check_post_emission_bundle(
            base_candidate.canonical_artifact().bytes(),
            base_candidate.source_map().bytes(),
            base_candidate.semantic_diff().bytes(),
            base_candidate.expected_semantic_diff_base(),
            FormatLimits::HARD,
        )
        .expect("base bundle");
        let target_checked = check_post_emission_bundle(
            target_candidate.canonical_artifact().bytes(),
            target_candidate.source_map().bytes(),
            target_candidate.semantic_diff().bytes(),
            target_candidate.expected_semantic_diff_base(),
            FormatLimits::HARD,
        )
        .expect("target bundle");
        let options = || {
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::RetainAvailable,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            )
        };
        let base = build_shared_network_revision(base_checked.canonical_network_input(), options())
            .expect("base revision");
        let target =
            build_shared_network_revision(target_checked.canonical_network_input(), options())
                .expect("target revision");
        let semantic_diff = target_candidate.semantic_diff().bytes().to_vec();
        let binding = SemanticDiffOriginBinding::new(
            SEMANTIC_DIFF_FORMAT_VERSION,
            target_candidate.semantic_diff().digest(),
            target_candidate.semantic_diff().byte_length(),
        );
        (base, target, semantic_diff, binding)
    }

    fn live_conflict_cutover_world(
        revision: Arc<SharedNetworkRevision>,
    ) -> (TrafficWorld, VehicleHandle) {
        let origin = *revision.canonical_origin();
        let mut world = TrafficWorld::install(
            Arc::clone(&revision),
            WorldConfig::new(4, 4, 64, 8, 1, 100),
            crate::cutover::tests::transaction_tests::source_for(
                origin,
                "fixture://live-conflict-cutover-base",
            ),
            284,
            crate::WorldPolicySelection::Pinned(crate::PolicyPin {
                policy: revision
                    .identity()
                    .stable_id(laneflow_static_contract::RightOfWayPolicySetOrdinal::from_raw(0))
                    .expect("fixture policy"),
            }),
        )
        .expect("install live Conflict world");
        let stream = revision
            .conflict()
            .participant_stream(ParticipantStreamOrdinal::from_raw(0))
            .expect("participant stream");
        let route = world
            .register_route(RouteRegisterInput::new(
                revision
                    .traffic()
                    .maneuvers()
                    .maneuver_path(stream.maneuver_path())
                    .expect("maneuver path")
                    .edges()
                    .to_vec(),
            ))
            .expect("conflict route");
        let vehicle = world
            .restore_unparked_vehicle(
                VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 0, 0),
                0,
                VehicleStatus::Active,
                None,
                None,
                true,
            )
            .expect("reservation vehicle");
        install_conflict_reservation(&mut world, route, vehicle);
        (world, vehicle)
    }

    #[test]
    fn live_conflict_reservation_survives_structural_revalidation() {
        let (world, vehicle) = world_with_conflict_reservation();
        let target = conflict_revision(true);
        let target_origin = *target.canonical_origin();
        let rebinding =
            CrossRevisionRebinding::build(world.binding.revision.identity(), target.identity())
                .expect("same semantics rebind");
        let candidate = migrate_structural_clone(
            &world,
            target,
            source_for(target_origin, "fixture://live-conflict-target"),
            &rebinding,
        )
        .expect("live reservation migrates before 3A revalidation");
        assert!(candidate.conflict_reservation(vehicle).is_some());
        assert_eq!(revalidate_vehicle_on(&candidate, vehicle), Ok(()));
        assert!(candidate.conflict_state_valid());
        let before = world.capture_snapshot().expect("source snapshot");
        let after = candidate.capture_snapshot().expect("target snapshot");
        assert_eq!(
            before.vehicles[vehicle.index() as usize].conflict_reservation,
            after.vehicles[vehicle.index() as usize].conflict_reservation
        );
        assert_eq!(before.conflict_lag_states, after.conflict_lag_states);
    }

    #[test]
    fn live_conflict_reservation_follows_stable_passage_across_cross_revision_static_insertion() {
        let (base, target, semantic_diff, semantic_diff_binding) =
            compile_conflict_cutover_test_pair(true, false);
        let (mut world, vehicle) = live_conflict_cutover_world(Arc::clone(&base));
        let source_reservation = world
            .conflict_reservation(vehicle)
            .expect("source reservation");
        let source_range = source_reservation.passage_range();
        let source_address = world
            .compiled_route(source_range.route())
            .expect("source compiled route")
            .conflicts[source_range.first_conflict_occurrence_index() as usize]
            .address();
        let source_locator = (
            base.identity()
                .stable_id(source_address.stream())
                .expect("source stream identity"),
            base.identity()
                .stable_id(source_address.zone())
                .expect("source zone identity"),
        );
        let descriptor = NetworkRevisionCutoverDescriptor::new(
            LfcaOriginBinding::from_canonical_origin(*base.canonical_origin()),
            LfcaOriginBinding::from_canonical_origin(*target.canonical_origin()),
            Some(semantic_diff_binding),
            MigrationPolicyKind::CrossRevisionDirect,
            world.world_binding(),
        );
        let transaction = world
            .prepare_cross_revision_cutover(
                Arc::clone(&target),
                crate::cutover::tests::transaction_tests::source_for(
                    *target.canonical_origin(),
                    "fixture://live-conflict-insertion-target",
                ),
                &descriptor,
                &semantic_diff,
                &CutoverPreflightLimits::new(1_048_576),
                &CutoverTransactionLimits::default(),
            )
            .expect("stable passage survives insertion during Prepare");
        let _commit = transaction
            .commit(&mut world)
            .expect("commit inserted passage cutover");

        let target_reservation = world
            .conflict_reservation(vehicle)
            .expect("target reservation");
        let target_range = target_reservation.passage_range();
        assert_eq!(target_range.passage_count(), source_range.passage_count());
        let target_address = world
            .compiled_route(target_range.route())
            .expect("target compiled route")
            .conflicts[target_range.first_conflict_occurrence_index() as usize]
            .address();
        assert_eq!(
            (
                target
                    .identity()
                    .stable_id(target_address.stream())
                    .expect("target stream identity"),
                target
                    .identity()
                    .stable_id(target_address.zone())
                    .expect("target zone identity"),
            ),
            source_locator,
            "reservation must follow stream/zone stable identity, not target table order"
        );
        assert_eq!(
            target
                .identity()
                .entity_count(laneflow_static_contract::EntityKind::ConflictZone),
            base.identity()
                .entity_count(laneflow_static_contract::EntityKind::ConflictZone)
                + 1
        );
        assert_eq!(
            target_reservation.acquired_tick(),
            source_reservation.acquired_tick()
        );
        assert!(world.conflict_state_valid());
        assert!(world.migration_journal().is_none());
    }

    #[test]
    fn live_conflict_reservation_rejects_same_stable_passage_with_changed_anchor_atomically() {
        let (base, target, semantic_diff, semantic_diff_binding) =
            compile_conflict_cutover_test_pair(false, true);
        let (mut world, vehicle) = live_conflict_cutover_world(Arc::clone(&base));
        let before = world
            .capture_snapshot()
            .expect("snapshot before failed cutover");
        let before_revision = *world.revision().canonical_origin();
        let target_origin = *target.canonical_origin();
        let descriptor = NetworkRevisionCutoverDescriptor::new(
            LfcaOriginBinding::from_canonical_origin(*base.canonical_origin()),
            LfcaOriginBinding::from_canonical_origin(target_origin),
            Some(semantic_diff_binding),
            MigrationPolicyKind::CrossRevisionDirect,
            world.world_binding(),
        );
        let error = match world.prepare_cross_revision_cutover(
            Arc::clone(&target),
            crate::cutover::tests::transaction_tests::source_for(
                target_origin,
                "fixture://live-conflict-anchor-target",
            ),
            &descriptor,
            &semantic_diff,
            &CutoverPreflightLimits::new(1_048_576),
            &CutoverTransactionLimits::default(),
        ) {
            Ok(_) => panic!("changed stable passage anchor must reject Prepare"),
            Err(error) => error,
        };
        assert_eq!(error, CutoverError::ConflictRevalidationFailed);
        assert_eq!(*world.revision().canonical_origin(), before_revision);
        assert_eq!(
            world
                .capture_snapshot()
                .expect("snapshot after failed cutover"),
            before
        );
        assert!(world.conflict_reservation(vehicle).is_some());
        assert!(world.migration_journal().is_none());
        let retry = match world.prepare_cross_revision_cutover(
            target,
            crate::cutover::tests::transaction_tests::source_for(
                target_origin,
                "fixture://live-conflict-anchor-retry",
            ),
            &descriptor,
            &semantic_diff,
            &CutoverPreflightLimits::new(1_048_576),
            &CutoverTransactionLimits::default(),
        ) {
            Ok(_) => panic!("changed stable passage anchor retry must reject Prepare"),
            Err(error) => error,
        };
        assert_eq!(retry, CutoverError::ConflictRevalidationFailed);
        assert!(world.migration_journal().is_none());
    }

    #[test]
    fn eligibility_clears_when_target_vehicle_is_no_longer_at_gate() {
        let (source, vehicle) = world_with_conflict_eligibility();
        let target_revision = conflict_revision(true);
        let target_origin = *target_revision.canonical_origin();
        let rebinding = CrossRevisionRebinding::build(
            source.binding.revision.identity(),
            target_revision.identity(),
        )
        .expect("same semantics rebind");
        let mut target = migrate_structural_clone(
            &source,
            target_revision,
            source_for(target_origin, "fixture://eligibility-position-target"),
            &rebinding,
        )
        .expect("baseline eligibility migration");
        target.committed.vehicles[vehicle.index() as usize]
            .state
            .as_mut()
            .expect("vehicle")
            .progress_mm -= 1;

        migrate_conflict_state(&source, &mut target, &rebinding, source.committed.time_ms)
            .expect("invalid target eligibility is cleared");
        assert!(target.committed.conflict_eligibility.is_empty());

        let mut projected = source.capture_snapshot().expect("source snapshot");
        project_expected_conflict(
            &source,
            &target,
            &rebinding,
            &mut projected,
            source.committed.time_ms,
        )
        .expect("independent projection clears invalid eligibility");
        assert!(
            projected.vehicles[vehicle.index() as usize]
                .conflict_eligibility
                .is_none()
        );
    }

    #[test]
    fn eligibility_clears_when_target_occurrence_is_not_mappable() {
        let (source, vehicle) = world_with_conflict_eligibility();
        let target_revision = conflict_revision(true);
        let target_origin = *target_revision.canonical_origin();
        let rebinding = CrossRevisionRebinding::build(
            source.binding.revision.identity(),
            target_revision.identity(),
        )
        .expect("same semantics rebind");
        let mut target = migrate_structural_clone(
            &source,
            target_revision,
            source_for(target_origin, "fixture://eligibility-occurrence-target"),
            &rebinding,
        )
        .expect("baseline eligibility migration");
        let route = target.vehicle_state(vehicle).expect("vehicle").route;
        target.committed.routes[route.index() as usize]
            .compiled
            .as_mut()
            .expect("compiled route")
            .conflicts
            .clear();

        migrate_conflict_state(&source, &mut target, &rebinding, source.committed.time_ms)
            .expect("unmappable eligibility is cleared");
        assert!(target.committed.conflict_eligibility.is_empty());

        let mut projected = source.capture_snapshot().expect("source snapshot");
        project_expected_conflict(
            &source,
            &target,
            &rebinding,
            &mut projected,
            source.committed.time_ms,
        )
        .expect("independent projection clears unmappable eligibility");
        assert!(
            projected.vehicles[vehicle.index() as usize]
                .conflict_eligibility
                .is_none()
        );
    }

    #[test]
    fn conflict_finalization_waits_for_removed_history_at_commit_time() {
        let mut target = installed_world(FULL_SPATIAL_LFCA, "fixture://finalization-time");
        let address = target
            .conflict_read()
            .addresses()
            .next()
            .expect("fixture conflict address");
        crate::conflict::ConflictWrite::new(
            &mut target.committed.conflict,
            &mut target.derived.conflict,
            &mut target.workspace.conflict,
        )
        .restore_lag_reference(address, crate::ConflictLagReference::CutoverFloor(100))
        .expect("seed Prepare floor");
        let plan = ConflictCutoverFinalizationPlan {
            floor_addresses: vec![address],
            removed_history_ready_at_ms: Some(500),
        };

        assert_eq!(
            finalize_conflict_cutover_floors(&mut target, &plan, 499),
            Err(CutoverError::ConflictRevalidationFailed)
        );
        assert_eq!(
            target.conflict_read().lag_reference(address),
            Some(crate::ConflictLagReference::CutoverFloor(100)),
            "failed finalization must not mutate any floor"
        );

        finalize_conflict_cutover_floors(&mut target, &plan, 500)
            .expect("history becomes eligible exactly at T_commit");
        assert_eq!(
            target.conflict_read().lag_reference(address),
            Some(crate::ConflictLagReference::CutoverFloor(500))
        );
    }

    #[test]
    fn conflict_migration_routes_every_scratch_reservation_through_staging_axis() {
        let (source, _) = crate::snapshot_restore::tests::world_with_conflict_reservation();
        let target = conflict_revision(true);
        let target_origin = *target.canonical_origin();
        let rebinding =
            CrossRevisionRebinding::build(source.binding.revision.identity(), target.identity())
                .expect("same-semantics rebind");
        let before = source
            .capture_snapshot()
            .expect("source before allocation failures");
        let mut fail_after = 0;
        loop {
            let result = with_staging_allocation_failure_after(fail_after, || {
                migrate_structural_clone(
                    &source,
                    Arc::clone(&target),
                    source_for(target_origin, "fixture://conflict-staging-target"),
                    &rebinding,
                )
            });
            match result {
                Err(error) => assert_eq!(
                    error,
                    CutoverError::StagingAllocFailed,
                    "Conflict staging allocation {fail_after}"
                ),
                Ok(candidate) => {
                    assert!(candidate.conflict_state_valid());
                    break;
                }
            }
            assert_eq!(source.capture_snapshot().expect("unchanged source"), before);
            fail_after += 1;
            assert!(fail_after < 64, "staging allocation enumeration terminates");
        }
        assert!(
            fail_after > 14,
            "Conflict migration exercised authority scratch"
        );
    }

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
        "../../laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfca"
    );

    #[test]
    fn signal_aspect_staging_reservation_failure_fails_closed() {
        // full-spatial 带信号组（group_count > 0）：重绑表已在外层构建，
        // 迁移内的首次切换暂存预留即候选信号灯色切片。
        let world = installed_world(FULL_SPATIAL_LFCA, "fixture://signal-stage");
        let target = revision(FULL_SPATIAL_LFCA);
        let target_origin = *target.canonical_origin();
        let rebinding =
            CrossRevisionRebinding::build(world.binding.revision.identity(), target.identity())
                .unwrap();
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

    #[test]
    fn each_structural_staging_reservation_fails_closed_and_can_retry() {
        let world = installed_world(FULL_SPATIAL_LFCA, "fixture://waiting-stage");
        let target = revision(FULL_SPATIAL_LFCA);
        let origin = *target.canonical_origin();
        let rebinding =
            CrossRevisionRebinding::build(world.binding.revision.identity(), target.identity())
                .expect("rebinding");
        let before = world.capture_snapshot().expect("before");
        // 前两次为 committed/next signal；随后覆盖 Waiting 权威、独立队列首尾及 scratch；
        // Conflict eligibility、authority 与 fixed-step scratch 继续走同一受检分配轴。
        for fail_after in 1..=24 {
            let result = with_staging_allocation_failure_after(fail_after, || {
                migrate_structural_clone(
                    &world,
                    Arc::clone(&target),
                    source_for(origin, "fixture://waiting-stage-target"),
                    &rebinding,
                )
            });
            assert_eq!(
                result.err(),
                Some(CutoverError::StagingAllocFailed),
                "structural allocation {fail_after}"
            );
            assert_eq!(world.capture_snapshot().expect("unchanged"), before);
        }
        let candidate = with_staging_allocation_failure_after(25, || {
            migrate_structural_clone(
                &world,
                target,
                source_for(origin, "fixture://waiting-stage-target"),
                &rebinding,
            )
        })
        .expect("all structural allocations succeeded");
        assert_eq!(
            candidate.committed.waiting_zones.len(),
            world.committed.waiting_zones.len()
        );
        assert_eq!(
            candidate.derived.waiting_links.len(),
            world.derived.waiting_links.len()
        );
        assert!(candidate.waiting_state_valid());
    }

    fn installed_world(bytes: &[u8], key: &str) -> TrafficWorld {
        let revision = revision(bytes);
        let origin = *revision.canonical_origin();
        TrafficWorld::install(
            std::sync::Arc::clone(&revision),
            WorldConfig::new(8, 4, 1_024, 1_024, 1, 100),
            source_for(origin, key),
            0,
            crate::test_policy::selection(&revision),
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
            .binding
            .revision
            .identity()
            .stable_id(edge)
            .expect("edge ordinal resolves")
            .as_untyped()
    }

    fn stable_space(world: &TrafficWorld, space: ParkingSpaceOrdinal) -> StableId128 {
        *world
            .binding
            .revision
            .identity()
            .stable_id(space)
            .expect("space ordinal resolves")
            .as_untyped()
    }

    fn stable_profile(world: &TrafficWorld, profile: VehicleProfileOrdinal) -> StableId128 {
        *world
            .binding
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
            std::sync::Arc::clone(&revision),
            WorldConfig::new(8, 4, 1_024, 1_024, 1, 100),
            source_for(origin, "fixture://parking-cutover-base"),
            0,
            crate::test_policy::selection(&revision),
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
                CrossRevisionRebinding::build(world.binding.revision.identity(), target.identity())
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
            assert_eq!(candidate.derived.active_order, [reserved]);
        }

        let unsafe_capacity = compiled_parking_revision(ParkingRevisionShape::Facility {
            capacity: 1,
            entry_progress_m: 20.0,
        });
        let unsafe_rebinding = CrossRevisionRebinding::build(
            world.binding.revision.identity(),
            unsafe_capacity.identity(),
        )
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
        let moved_rebinding = CrossRevisionRebinding::build(
            world.binding.revision.identity(),
            moved_anchor.identity(),
        )
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
                CrossRevisionRebinding::build(world.binding.revision.identity(), target.identity())
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
        let rebinding = CrossRevisionRebinding::build(
            world.binding.revision.identity(),
            target_revision.identity(),
        )
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
        let route_capacity = usize::try_from(candidate.binding.config.route_capacity()).unwrap();
        let vehicle_capacity =
            usize::try_from(candidate.binding.config.vehicle_capacity()).unwrap();
        assert!(candidate.committed.routes.capacity() >= route_capacity);
        assert!(candidate.committed.free_routes.capacity() >= route_capacity);
        assert!(candidate.committed.vehicles.capacity() >= vehicle_capacity);
        assert!(candidate.committed.free_vehicles.capacity() >= vehicle_capacity);
        assert!(candidate.committed.live_order.capacity() >= vehicle_capacity);
        assert!(candidate.derived.active_order.capacity() >= vehicle_capacity);
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
        let rebinding = CrossRevisionRebinding::build(
            world.binding.revision.identity(),
            target_revision.identity(),
        )
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
        candidate.committed.vehicles[index]
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
        candidate.committed.vehicles[index]
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
        let rebinding = CrossRevisionRebinding::build(
            world.binding.revision.identity(),
            target_revision.identity(),
        )
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
        world.committed.vehicles[completed_index]
            .state
            .as_mut()
            .expect("completed")
            .status = VehicleStatus::Completed;
        world.rebuild_active_order();
        assert!(!world.derived.active_order.contains(&completed));
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
        assert!(Arc::ptr_eq(&candidate.binding.revision, &target_revision));
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
        let rebinding = CrossRevisionRebinding::build(
            world.binding.revision.identity(),
            rebinding_probe.identity(),
        )
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
        let rebinding = CrossRevisionRebinding::build(
            world.binding.revision.identity(),
            target_revision.identity(),
        )
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
        let rebinding = CrossRevisionRebinding::build(
            world.binding.revision.identity(),
            target_revision.identity(),
        )
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
        let rebinding = CrossRevisionRebinding::build(
            world.binding.revision.identity(),
            target_revision.identity(),
        )
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
        let rebinding = CrossRevisionRebinding::build(
            world.binding.revision.identity(),
            target_revision.identity(),
        )
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
        let rebinding = CrossRevisionRebinding::build(
            world.binding.revision.identity(),
            target_revision.identity(),
        )
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
        world.committed.vehicles[index]
            .state
            .as_mut()
            .expect("vehicle")
            .class = other_class;
        let target_revision = revision(TARGET);
        let rebinding = CrossRevisionRebinding::build(
            world.binding.revision.identity(),
            target_revision.identity(),
        )
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
        let rebinding = CrossRevisionRebinding::build(
            world.binding.revision.identity(),
            target_revision.identity(),
        )
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
