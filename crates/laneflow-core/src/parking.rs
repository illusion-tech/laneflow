//! 已完成 Core domain normalization 的 immutable static Parking registry。

use std::f64::consts::PI;

use indexmap::IndexMap;

use crate::{
    error::CoreError,
    graph::{EDGE_BOUNDARY_EPSILON, LaneGraph},
    handle::{
        EdgeHandle, ParkingAreaHandle, ParkingSpaceHandle, RouteHandle, VehicleHandle,
        VehicleProfileHandle,
    },
    id::validate_external_id,
    profile::GEOMETRY_GAP_EPSILON,
    vehicle::{Speed, VehicleStatus},
    world::CoreWorld,
};

/// Parking anchor 的语义位置。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ParkingAnchorKind {
    Entry,
    Exit,
}

/// optional ParkingSpace 分组。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParkingArea {
    id: String,
}

impl ParkingArea {
    /// 创建尚未 normalization 的 area definition。
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }

    /// 返回 area external ID。
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// ParkingSpace 的 edge-relative rectangular pose。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParkingSpaceGeometry {
    lateral_offset: f64,
    heading_offset_radians: f64,
    length: f64,
    width: f64,
}

impl ParkingSpaceGeometry {
    /// 创建尚未 normalization 的 geometry value。
    pub const fn new(
        lateral_offset: f64,
        heading_offset_radians: f64,
        length: f64,
        width: f64,
    ) -> Self {
        Self {
            lateral_offset,
            heading_offset_radians,
            length,
            width,
        }
    }

    pub const fn lateral_offset(self) -> f64 {
        self.lateral_offset
    }

    pub const fn heading_offset_radians(self) -> f64 {
        self.heading_offset_radians
    }

    pub const fn length(self) -> f64 {
        self.length
    }

    pub const fn width(self) -> f64 {
        self.width
    }
}

/// 已解析到当前 LaneGraph 的 Parking entry/exit anchor。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParkingLaneAnchor {
    edge: EdgeHandle,
    progress: f64,
}

impl ParkingLaneAnchor {
    const fn new(edge: EdgeHandle, progress: f64) -> Self {
        Self { edge, progress }
    }

    pub const fn edge(self) -> EdgeHandle {
        self.edge
    }

    pub const fn progress(self) -> f64 {
        self.progress
    }
}

/// 尚未解析 graph handles 的 static ParkingSpace definition。
#[derive(Clone, Debug, PartialEq)]
pub struct ParkingSpace {
    id: String,
    area_id: Option<String>,
    entry_edge_id: String,
    entry_progress: f64,
    exit_edge_id: String,
    exit_progress: f64,
    geometry: ParkingSpaceGeometry,
}

impl ParkingSpace {
    /// 创建尚未 normalization 的 space definition。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        area_id: Option<String>,
        entry_edge_id: impl Into<String>,
        entry_progress: f64,
        exit_edge_id: impl Into<String>,
        exit_progress: f64,
        geometry: ParkingSpaceGeometry,
    ) -> Self {
        Self {
            id: id.into(),
            area_id,
            entry_edge_id: entry_edge_id.into(),
            entry_progress,
            exit_edge_id: exit_edge_id.into(),
            exit_progress,
            geometry,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn area_id(&self) -> Option<&str> {
        self.area_id.as_deref()
    }

    pub fn entry_edge_id(&self) -> &str {
        &self.entry_edge_id
    }

    pub const fn entry_progress(&self) -> f64 {
        self.entry_progress
    }

    pub fn exit_edge_id(&self) -> &str {
        &self.exit_edge_id
    }

    pub const fn exit_progress(&self) -> f64 {
        self.exit_progress
    }

    pub const fn geometry(&self) -> ParkingSpaceGeometry {
        self.geometry
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ResolvedParkingSpace {
    definition: ParkingSpace,
    area: Option<ParkingAreaHandle>,
    entry: ParkingLaneAnchor,
    exit: ParkingLaneAnchor,
}

/// Static Parking definitions、handles 与 ordered reverse indexes。
#[derive(Clone, Debug, PartialEq)]
pub struct ParkingRegistry {
    areas: Vec<ParkingArea>,
    area_handles: IndexMap<String, ParkingAreaHandle>,
    area_spaces: Vec<Vec<ParkingSpaceHandle>>,
    spaces: Vec<ResolvedParkingSpace>,
    space_handles: IndexMap<String, ParkingSpaceHandle>,
}

impl ParkingRegistry {
    /// 创建合法的空 Parking registry。
    pub fn empty() -> Self {
        Self {
            areas: Vec::new(),
            area_handles: IndexMap::new(),
            area_spaces: Vec::new(),
            spaces: Vec::new(),
            space_handles: IndexMap::new(),
        }
    }

    /// 按固定 fail-fast 顺序 normalization 全部 static Parking definitions。
    pub fn try_new<A, S>(lane_graph: &LaneGraph, areas: A, spaces: S) -> Result<Self, CoreError>
    where
        A: IntoIterator<Item = ParkingArea>,
        S: IntoIterator<Item = ParkingSpace>,
    {
        let mut area_definitions = Vec::new();
        let mut area_handles = IndexMap::new();
        for area in areas {
            validate_external_id("parking.areas[].id", area.id())?;
            if area_handles.contains_key(area.id()) {
                return Err(CoreError::DuplicateParkingAreaId {
                    area_id: area.id().to_owned(),
                });
            }
            let handle = ParkingAreaHandle::new(area_definitions.len());
            area_handles.insert(area.id().to_owned(), handle);
            area_definitions.push(area);
        }

        let mut space_definitions = Vec::new();
        let mut space_handles = IndexMap::new();
        let mut space_areas = Vec::new();
        let mut area_has_member = vec![false; area_definitions.len()];
        for space in spaces {
            validate_external_id("parking.spaces[].id", space.id())?;
            if space_handles.contains_key(space.id()) {
                return Err(CoreError::DuplicateParkingSpaceId {
                    space_id: space.id().to_owned(),
                });
            }
            let area = if let Some(area_id) = space.area_id() {
                validate_external_id("parking.spaces[].areaId", area_id)?;
                let handle = area_handles.get(area_id).copied().ok_or_else(|| {
                    CoreError::UnknownParkingSpaceArea {
                        space_id: space.id().to_owned(),
                        area_id: area_id.to_owned(),
                    }
                })?;
                area_has_member[handle.index()] = true;
                Some(handle)
            } else {
                None
            };
            let handle = ParkingSpaceHandle::new(space_definitions.len());
            space_handles.insert(space.id().to_owned(), handle);
            space_areas.push(area);
            space_definitions.push(space);
        }

        let mut entries = Vec::with_capacity(space_definitions.len());
        let mut exits = Vec::with_capacity(space_definitions.len());
        for space in &space_definitions {
            entries.push(resolve_anchor(
                lane_graph,
                space,
                ParkingAnchorKind::Entry,
                space.entry_edge_id(),
                space.entry_progress(),
            )?);
            exits.push(resolve_anchor(
                lane_graph,
                space,
                ParkingAnchorKind::Exit,
                space.exit_edge_id(),
                space.exit_progress(),
            )?);
        }

        for space in &space_definitions {
            validate_geometry(space)?;
        }

        for (index, has_member) in area_has_member.iter().copied().enumerate() {
            if !has_member {
                return Err(CoreError::OrphanParkingArea {
                    area_id: area_definitions[index].id().to_owned(),
                });
            }
        }

        let mut area_spaces = vec![Vec::new(); area_definitions.len()];
        for (index, area) in space_areas.iter().copied().enumerate() {
            if let Some(area) = area {
                area_spaces[area.index()].push(ParkingSpaceHandle::new(index));
            }
        }

        let spaces = space_definitions
            .into_iter()
            .zip(space_areas)
            .zip(entries)
            .zip(exits)
            .map(|(((definition, area), entry), exit)| ResolvedParkingSpace {
                definition,
                area,
                entry,
                exit,
            })
            .collect();

        Ok(Self {
            areas: area_definitions,
            area_handles,
            area_spaces,
            spaces,
            space_handles,
        })
    }

    pub(crate) fn rebind_to_lane_graph(self, lane_graph: &LaneGraph) -> Result<Self, CoreError> {
        Self::try_new(
            lane_graph,
            self.areas,
            self.spaces.into_iter().map(|resolved| resolved.definition),
        )
    }

    pub fn is_empty(&self) -> bool {
        self.areas.is_empty() && self.spaces.is_empty()
    }

    pub fn area_handle(&self, id: &str) -> Option<ParkingAreaHandle> {
        self.area_handles.get(id).copied()
    }

    pub fn area(&self, handle: ParkingAreaHandle) -> Option<&ParkingArea> {
        self.areas.get(handle.index())
    }

    pub fn area_external_id(&self, handle: ParkingAreaHandle) -> Option<&str> {
        self.area(handle).map(ParkingArea::id)
    }

    pub fn areas(&self) -> impl ExactSizeIterator<Item = &ParkingArea> {
        self.areas.iter()
    }

    pub fn space_handle(&self, id: &str) -> Option<ParkingSpaceHandle> {
        self.space_handles.get(id).copied()
    }

    pub fn space(&self, handle: ParkingSpaceHandle) -> Option<&ParkingSpace> {
        self.spaces
            .get(handle.index())
            .map(|resolved| &resolved.definition)
    }

    pub fn space_external_id(&self, handle: ParkingSpaceHandle) -> Option<&str> {
        self.space(handle).map(ParkingSpace::id)
    }

    pub fn spaces(&self) -> impl ExactSizeIterator<Item = &ParkingSpace> {
        self.spaces.iter().map(|resolved| &resolved.definition)
    }

    pub fn area_spaces(&self, handle: ParkingAreaHandle) -> Option<&[ParkingSpaceHandle]> {
        self.area_spaces.get(handle.index()).map(Vec::as_slice)
    }

    /// 返回 `Some(None)` 表示合法 standalone space，`None` 表示未知 handle。
    pub fn space_area(&self, handle: ParkingSpaceHandle) -> Option<Option<ParkingAreaHandle>> {
        self.spaces
            .get(handle.index())
            .map(|resolved| resolved.area)
    }

    pub fn space_entry(&self, handle: ParkingSpaceHandle) -> Option<ParkingLaneAnchor> {
        self.spaces
            .get(handle.index())
            .map(|resolved| resolved.entry)
    }

    pub fn space_exit(&self, handle: ParkingSpaceHandle) -> Option<ParkingLaneAnchor> {
        self.spaces
            .get(handle.index())
            .map(|resolved| resolved.exit)
    }

    pub fn space_geometry(&self, handle: ParkingSpaceHandle) -> Option<ParkingSpaceGeometry> {
        self.space(handle).map(ParkingSpace::geometry)
    }
}

impl Default for ParkingRegistry {
    fn default() -> Self {
        Self::empty()
    }
}

fn resolve_anchor(
    lane_graph: &LaneGraph,
    space: &ParkingSpace,
    anchor: ParkingAnchorKind,
    edge_id: &str,
    edge_progress: f64,
) -> Result<ParkingLaneAnchor, CoreError> {
    let field = match anchor {
        ParkingAnchorKind::Entry => "parking.spaces[].entry.edgeId",
        ParkingAnchorKind::Exit => "parking.spaces[].exit.edgeId",
    };
    validate_external_id(field, edge_id)?;
    let edge =
        lane_graph
            .edge_handle(edge_id)
            .ok_or_else(|| CoreError::UnknownParkingAnchorEdge {
                space_id: space.id().to_owned(),
                anchor,
                edge_id: edge_id.to_owned(),
            })?;
    let edge_length = lane_graph
        .edge_length(edge)
        .expect("resolved parking anchor edge must have length")
        .value();
    if !edge_progress.is_finite()
        || edge_progress <= EDGE_BOUNDARY_EPSILON
        || edge_progress >= edge_length - EDGE_BOUNDARY_EPSILON
    {
        return Err(CoreError::ParkingAnchorProgressOutOfRange {
            space_id: space.id().to_owned(),
            anchor,
            edge_id: edge_id.to_owned(),
            edge_progress,
            edge_length,
        });
    }
    Ok(ParkingLaneAnchor::new(edge, edge_progress))
}

fn validate_geometry(space: &ParkingSpace) -> Result<(), CoreError> {
    let geometry = space.geometry();
    let values = [
        (
            "lateralOffset",
            geometry.lateral_offset(),
            geometry.lateral_offset().is_finite()
                && geometry.lateral_offset().abs() > GEOMETRY_GAP_EPSILON,
            "必须是 finite 且绝对值大于 GEOMETRY_GAP_EPSILON",
        ),
        (
            "headingOffsetRadians",
            geometry.heading_offset_radians(),
            geometry.heading_offset_radians().is_finite()
                && geometry.heading_offset_radians() >= -PI
                && geometry.heading_offset_radians() < PI,
            "必须是 finite 且位于 [-PI, PI)",
        ),
        (
            "length",
            geometry.length(),
            geometry.length().is_finite() && geometry.length() > GEOMETRY_GAP_EPSILON,
            "必须是 finite 且大于 GEOMETRY_GAP_EPSILON",
        ),
        (
            "width",
            geometry.width(),
            geometry.width().is_finite() && geometry.width() > GEOMETRY_GAP_EPSILON,
            "必须是 finite 且大于 GEOMETRY_GAP_EPSILON",
        ),
    ];
    for (field, value, valid, requirement) in values {
        if !valid {
            return Err(CoreError::InvalidParkingGeometryValue {
                space_id: space.id().to_owned(),
                field,
                value,
                requirement,
            });
        }
    }
    Ok(())
}

/// Parking lifecycle command identity。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ParkingCommandKind {
    Reserve,
    CancelReservation,
    Commit,
    Leave,
    RebindReservedVehicleRoute,
    SpawnParkedVehicle,
}

/// Committed ParkingSpace runtime state。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParkingSpaceState {
    Vacant,
    Reserved { vehicle: VehicleHandle },
    Occupied { vehicle: VehicleHandle },
}

/// Reserved vehicle 相对 selected entry 的 committed approach 状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParkingApproachState {
    Dormant,
    Approaching {
        route: RouteHandle,
        route_edge_index: usize,
    },
    Arrived {
        route: RouteHandle,
        route_edge_index: usize,
    },
}

/// 单个 live vehicle 的 Parking binding view。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum VehicleParkingState {
    Unbound,
    Reserved {
        space: ParkingSpaceHandle,
        approach: ParkingApproachState,
    },
    Occupied {
        space: ParkingSpaceHandle,
    },
}

/// Global 或 ParkingArea 的 committed capacity/count view。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParkingCounts {
    pub capacity: usize,
    pub vacant: usize,
    pub reserved: usize,
    pub occupied: usize,
}

impl ParkingCounts {
    /// `available` 与 `vacant` 同义。
    pub const fn available(self) -> usize {
        self.vacant
    }
}

/// 同步 command 是否改变了 committed authority。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParkingCommandEffect {
    Applied,
    AlreadySatisfied,
}

/// Parking forward/reverse binding 类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParkingBindingKind {
    Reserved,
    Occupied,
}

/// Parking binding 被 lifecycle cleanup 释放的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParkingReleaseReason {
    RouteCompleted,
    VehicleDespawn,
}

/// Reservation command result。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParkingReservationRecord {
    pub vehicle: VehicleHandle,
    pub space: ParkingSpaceHandle,
    pub effect: ParkingCommandEffect,
}

/// Pair-specific reservation cancellation result。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParkingReservationCancellationRecord {
    pub vehicle: VehicleHandle,
    pub space: ParkingSpaceHandle,
    pub effect: ParkingCommandEffect,
}

/// Explicit park commit result。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParkingCommitRecord {
    pub vehicle: VehicleHandle,
    pub space: ParkingSpaceHandle,
    pub effect: ParkingCommandEffect,
}

/// Leave command input；progress 固定来自 ParkingSpace exit anchor。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LeaveParkingInput {
    pub vehicle: VehicleHandle,
    pub space: ParkingSpaceHandle,
    pub route: RouteHandle,
    pub route_edge_index: usize,
}

/// Successful/no-op leave result。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParkingLeaveRecord {
    pub vehicle: VehicleHandle,
    pub space: ParkingSpaceHandle,
    pub route: RouteHandle,
    pub route_edge_index: usize,
    pub effect: ParkingCommandEffect,
}

/// Reserved vehicle route rebind input。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RebindReservedVehicleRouteInput {
    pub vehicle: VehicleHandle,
    pub space: ParkingSpaceHandle,
    pub route: RouteHandle,
    pub route_edge_index: usize,
}

/// Reserved vehicle route rebind result。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReservedVehicleRouteRebindRecord {
    pub vehicle: VehicleHandle,
    pub space: ParkingSpaceHandle,
    pub from_route: RouteHandle,
    pub from_route_edge_index: usize,
    pub to_route: RouteHandle,
    pub to_route_edge_index: usize,
    pub effect: ParkingCommandEffect,
}

/// Atomic parked spawn input。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParkedVehicleSpawnInput {
    pub id: String,
    pub profile: VehicleProfileHandle,
    pub route_id: String,
    pub route_edge_index: usize,
    pub space: ParkingSpaceHandle,
}

/// Atomic parked spawn result。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParkedVehicleSpawnRecord {
    pub vehicle: VehicleHandle,
    pub space: ParkingSpaceHandle,
    pub route: RouteHandle,
    pub route_edge_index: usize,
}

/// Lifecycle cleanup 释放的 Parking binding。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParkingReleaseRecord {
    pub vehicle: VehicleHandle,
    pub space: ParkingSpaceHandle,
    pub previous_binding: ParkingBindingKind,
    pub reason: ParkingReleaseReason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ParkingApproachTarget {
    pub(crate) route: RouteHandle,
    pub(crate) route_edge_index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RuntimeVehicleParkingBinding {
    Reserved {
        vehicle: VehicleHandle,
        space: ParkingSpaceHandle,
        target: Option<ParkingApproachTarget>,
    },
    Occupied {
        vehicle: VehicleHandle,
        space: ParkingSpaceHandle,
    },
}

impl RuntimeVehicleParkingBinding {
    const fn vehicle(self) -> VehicleHandle {
        match self {
            Self::Reserved { vehicle, .. } | Self::Occupied { vehicle, .. } => vehicle,
        }
    }

    pub(crate) const fn space(self) -> ParkingSpaceHandle {
        match self {
            Self::Reserved { space, .. } | Self::Occupied { space, .. } => space,
        }
    }

    pub(crate) const fn kind(self) -> ParkingBindingKind {
        match self {
            Self::Reserved { .. } => ParkingBindingKind::Reserved,
            Self::Occupied { .. } => ParkingBindingKind::Occupied,
        }
    }
}

/// Core-owned mutable Parking authority。Public callers只能通过 `ParkingSnapshot` 读取。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ParkingRuntimeState {
    spaces: Vec<ParkingSpaceState>,
    vehicle_bindings: Vec<Option<RuntimeVehicleParkingBinding>>,
    global_counts: ParkingCounts,
    area_counts: Vec<ParkingCounts>,
}

impl ParkingRuntimeState {
    pub(crate) fn new(registry: &ParkingRegistry) -> Self {
        let area_counts = registry
            .area_spaces
            .iter()
            .map(|members| ParkingCounts {
                capacity: members.len(),
                vacant: members.len(),
                reserved: 0,
                occupied: 0,
            })
            .collect::<Vec<_>>();
        let capacity = registry.spaces.len();
        Self {
            spaces: vec![ParkingSpaceState::Vacant; capacity],
            vehicle_bindings: Vec::new(),
            global_counts: ParkingCounts {
                capacity,
                vacant: capacity,
                reserved: 0,
                occupied: 0,
            },
            area_counts,
        }
    }

    pub(crate) fn validate_step_sentinel(
        &self,
        registry: &ParkingRegistry,
    ) -> Result<(), CoreError> {
        let valid = self.spaces.len() == registry.spaces.len()
            && self.area_counts.len() == registry.areas.len()
            && self.global_counts.capacity == self.spaces.len()
            && self.global_counts.vacant
                + self.global_counts.reserved
                + self.global_counts.occupied
                == self.global_counts.capacity;
        if valid {
            Ok(())
        } else {
            Err(CoreError::ParkingBindingInvariantViolation {
                stage: "step_sentinel",
                vehicle: None,
                space: None,
            })
        }
    }

    pub(crate) const fn reserved_count(&self) -> usize {
        self.global_counts.reserved
    }

    #[cfg(test)]
    pub(crate) fn assert_consistent(
        &self,
        registry: &ParkingRegistry,
        mut vehicle_status: impl FnMut(VehicleHandle) -> Option<VehicleStatus>,
    ) {
        assert_eq!(self.spaces.len(), registry.spaces.len());
        assert_eq!(self.area_counts.len(), registry.areas.len());

        let mut expected_global = ParkingCounts {
            capacity: self.spaces.len(),
            vacant: 0,
            reserved: 0,
            occupied: 0,
        };
        let mut expected_areas = registry
            .area_spaces
            .iter()
            .map(|members| ParkingCounts {
                capacity: members.len(),
                vacant: 0,
                reserved: 0,
                occupied: 0,
            })
            .collect::<Vec<_>>();

        for (space_index, state) in self.spaces.iter().copied().enumerate() {
            let space = ParkingSpaceHandle::new(space_index);
            let area = registry.spaces[space_index].area;
            let count = |counts: &mut ParkingCounts| match state {
                ParkingSpaceState::Vacant => counts.vacant += 1,
                ParkingSpaceState::Reserved { .. } => counts.reserved += 1,
                ParkingSpaceState::Occupied { .. } => counts.occupied += 1,
            };
            count(&mut expected_global);
            if let Some(area) = area {
                count(&mut expected_areas[area.index()]);
            }

            match state {
                ParkingSpaceState::Vacant => {
                    assert!(
                        self.vehicle_bindings
                            .iter()
                            .flatten()
                            .all(|binding| { binding.space() != space })
                    );
                }
                ParkingSpaceState::Reserved { vehicle } => {
                    assert!(matches!(
                        self.vehicle_binding(vehicle),
                        Some(RuntimeVehicleParkingBinding::Reserved {
                            vehicle: bound_vehicle,
                            space: bound_space,
                            ..
                        }) if bound_vehicle == vehicle && bound_space == space
                    ));
                    assert_eq!(vehicle_status(vehicle), Some(VehicleStatus::Active));
                }
                ParkingSpaceState::Occupied { vehicle } => {
                    assert_eq!(
                        self.vehicle_binding(vehicle),
                        Some(RuntimeVehicleParkingBinding::Occupied { vehicle, space })
                    );
                    assert_eq!(vehicle_status(vehicle), Some(VehicleStatus::Parked));
                }
            }
        }

        for binding in self.vehicle_bindings.iter().flatten().copied() {
            let vehicle = binding.vehicle();
            let expected_space_state = match binding {
                RuntimeVehicleParkingBinding::Reserved { .. } => {
                    assert_eq!(vehicle_status(vehicle), Some(VehicleStatus::Active));
                    ParkingSpaceState::Reserved { vehicle }
                }
                RuntimeVehicleParkingBinding::Occupied { .. } => {
                    assert_eq!(vehicle_status(vehicle), Some(VehicleStatus::Parked));
                    ParkingSpaceState::Occupied { vehicle }
                }
            };
            assert_eq!(
                self.space_state(binding.space()),
                Some(expected_space_state)
            );
        }

        assert_eq!(self.global_counts, expected_global);
        assert_eq!(self.area_counts, expected_areas);
    }

    #[cfg(test)]
    pub(crate) fn corrupt_global_capacity_for_test(&mut self) {
        self.global_counts.capacity = self.global_counts.capacity.saturating_add(1);
    }

    #[cfg(test)]
    pub(crate) fn retained_bytes(&self) -> usize {
        self.spaces.capacity() * std::mem::size_of::<ParkingSpaceState>()
            + self.vehicle_bindings.capacity()
                * std::mem::size_of::<Option<RuntimeVehicleParkingBinding>>()
            + self.area_counts.capacity() * std::mem::size_of::<ParkingCounts>()
    }

    pub(crate) fn space_state(&self, space: ParkingSpaceHandle) -> Option<ParkingSpaceState> {
        self.spaces.get(space.index()).copied()
    }

    pub(crate) fn vehicle_binding(
        &self,
        vehicle: VehicleHandle,
    ) -> Option<RuntimeVehicleParkingBinding> {
        self.vehicle_bindings
            .get(vehicle.index())
            .copied()
            .flatten()
            .filter(|binding| binding.vehicle() == vehicle)
    }

    pub(crate) fn prepare_vehicle_slot(&mut self, vehicle_index: usize) {
        self.vehicle_bindings
            .reserve((vehicle_index + 1).saturating_sub(self.vehicle_bindings.len()));
    }

    pub(crate) fn register_unbound_vehicle(&mut self, vehicle: VehicleHandle) {
        self.vehicle_bindings
            .resize(self.vehicle_bindings.len().max(vehicle.index() + 1), None);
        debug_assert!(self.vehicle_bindings[vehicle.index()].is_none());
    }

    pub(crate) fn reserve(
        &mut self,
        registry: &ParkingRegistry,
        vehicle: VehicleHandle,
        space: ParkingSpaceHandle,
        target: Option<ParkingApproachTarget>,
    ) {
        self.register_unbound_vehicle(vehicle);
        debug_assert_eq!(self.spaces[space.index()], ParkingSpaceState::Vacant);
        self.spaces[space.index()] = ParkingSpaceState::Reserved { vehicle };
        self.vehicle_bindings[vehicle.index()] = Some(RuntimeVehicleParkingBinding::Reserved {
            vehicle,
            space,
            target,
        });
        self.update_counts(registry, space, ParkingBindingKind::Reserved, true);
    }

    pub(crate) fn cancel(
        &mut self,
        registry: &ParkingRegistry,
        vehicle: VehicleHandle,
        space: ParkingSpaceHandle,
    ) {
        debug_assert_eq!(
            self.spaces[space.index()],
            ParkingSpaceState::Reserved { vehicle }
        );
        self.spaces[space.index()] = ParkingSpaceState::Vacant;
        self.vehicle_bindings[vehicle.index()] = None;
        self.update_counts(registry, space, ParkingBindingKind::Reserved, false);
    }

    pub(crate) fn commit(
        &mut self,
        registry: &ParkingRegistry,
        vehicle: VehicleHandle,
        space: ParkingSpaceHandle,
    ) {
        debug_assert_eq!(
            self.spaces[space.index()],
            ParkingSpaceState::Reserved { vehicle }
        );
        self.spaces[space.index()] = ParkingSpaceState::Occupied { vehicle };
        self.vehicle_bindings[vehicle.index()] =
            Some(RuntimeVehicleParkingBinding::Occupied { vehicle, space });
        self.transition_reserved_to_occupied(registry, space);
    }

    pub(crate) fn occupy_new(
        &mut self,
        registry: &ParkingRegistry,
        vehicle: VehicleHandle,
        space: ParkingSpaceHandle,
    ) {
        self.register_unbound_vehicle(vehicle);
        debug_assert_eq!(self.spaces[space.index()], ParkingSpaceState::Vacant);
        self.spaces[space.index()] = ParkingSpaceState::Occupied { vehicle };
        self.vehicle_bindings[vehicle.index()] =
            Some(RuntimeVehicleParkingBinding::Occupied { vehicle, space });
        self.update_counts(registry, space, ParkingBindingKind::Occupied, true);
    }

    pub(crate) fn release(
        &mut self,
        registry: &ParkingRegistry,
        vehicle: VehicleHandle,
    ) -> Option<(ParkingSpaceHandle, ParkingBindingKind)> {
        let binding = self.vehicle_binding(vehicle)?;
        let space = binding.space();
        let kind = binding.kind();
        self.spaces[space.index()] = ParkingSpaceState::Vacant;
        self.vehicle_bindings[vehicle.index()] = None;
        self.update_counts(registry, space, kind, false);
        Some((space, kind))
    }

    pub(crate) fn rebind_target(&mut self, vehicle: VehicleHandle, target: ParkingApproachTarget) {
        let Some(RuntimeVehicleParkingBinding::Reserved {
            vehicle: bound_vehicle,
            space,
            ..
        }) = self.vehicle_binding(vehicle)
        else {
            unreachable!("reserved rebind must have exact binding")
        };
        self.vehicle_bindings[vehicle.index()] = Some(RuntimeVehicleParkingBinding::Reserved {
            vehicle: bound_vehicle,
            space,
            target: Some(target),
        });
    }

    fn update_counts(
        &mut self,
        registry: &ParkingRegistry,
        space: ParkingSpaceHandle,
        binding: ParkingBindingKind,
        bind: bool,
    ) {
        update_one_count(&mut self.global_counts, binding, bind);
        if let Some(area) = registry.spaces[space.index()].area {
            update_one_count(&mut self.area_counts[area.index()], binding, bind);
        }
    }

    fn transition_reserved_to_occupied(
        &mut self,
        registry: &ParkingRegistry,
        space: ParkingSpaceHandle,
    ) {
        transition_one_count(&mut self.global_counts);
        if let Some(area) = registry.spaces[space.index()].area {
            transition_one_count(&mut self.area_counts[area.index()]);
        }
    }
}

fn update_one_count(counts: &mut ParkingCounts, binding: ParkingBindingKind, bind: bool) {
    if bind {
        counts.vacant = counts
            .vacant
            .checked_sub(1)
            .expect("vacant count underflow");
        match binding {
            ParkingBindingKind::Reserved => counts.reserved += 1,
            ParkingBindingKind::Occupied => counts.occupied += 1,
        }
    } else {
        counts.vacant += 1;
        match binding {
            ParkingBindingKind::Reserved => {
                counts.reserved = counts.reserved.checked_sub(1).expect("reserved underflow");
            }
            ParkingBindingKind::Occupied => {
                counts.occupied = counts.occupied.checked_sub(1).expect("occupied underflow");
            }
        }
    }
}

fn transition_one_count(counts: &mut ParkingCounts) {
    counts.reserved = counts.reserved.checked_sub(1).expect("reserved underflow");
    counts.occupied += 1;
}

/// 借用同一个 committed `CoreWorld` 的 immutable Parking view。
#[derive(Clone, Copy)]
pub struct ParkingSnapshot<'a> {
    world: &'a CoreWorld,
}

impl<'a> ParkingSnapshot<'a> {
    pub(crate) const fn new(world: &'a CoreWorld) -> Self {
        Self { world }
    }

    pub fn space_state(&self, space: ParkingSpaceHandle) -> Option<ParkingSpaceState> {
        self.world.parking_runtime.space_state(space)
    }

    pub fn vehicle_state(&self, vehicle: VehicleHandle) -> Option<VehicleParkingState> {
        let state = self.world.vehicle(vehicle)?;
        let binding = self.world.parking_runtime.vehicle_binding(vehicle);
        Some(match binding {
            None => VehicleParkingState::Unbound,
            Some(RuntimeVehicleParkingBinding::Occupied { space, .. }) => {
                VehicleParkingState::Occupied { space }
            }
            Some(RuntimeVehicleParkingBinding::Reserved { space, target, .. }) => {
                let approach =
                    match target {
                        None => ParkingApproachState::Dormant,
                        Some(target)
                            if state.status == VehicleStatus::Active
                                && state.route == target.route
                                && state.route_edge_index == target.route_edge_index
                                && self.world.parking().space_entry(space).is_some_and(
                                    |entry| {
                                        (state.edge_progress.value() - entry.progress()).abs()
                                            <= EDGE_BOUNDARY_EPSILON
                                            && state.current_speed == Speed::ZERO
                                    },
                                ) =>
                        {
                            ParkingApproachState::Arrived {
                                route: target.route,
                                route_edge_index: target.route_edge_index,
                            }
                        }
                        Some(target) => ParkingApproachState::Approaching {
                            route: target.route,
                            route_edge_index: target.route_edge_index,
                        },
                    };
                VehicleParkingState::Reserved { space, approach }
            }
        })
    }

    pub const fn counts(&self) -> ParkingCounts {
        self.world.parking_runtime.global_counts
    }

    pub fn area_counts(&self, area: ParkingAreaHandle) -> Option<ParkingCounts> {
        self.world
            .parking_runtime
            .area_counts
            .get(area.index())
            .copied()
    }

    pub fn space_states(
        &self,
    ) -> impl ExactSizeIterator<Item = (ParkingSpaceHandle, ParkingSpaceState)> + '_ {
        self.world
            .parking_runtime
            .spaces
            .iter()
            .copied()
            .enumerate()
            .map(|(index, state)| (ParkingSpaceHandle::new(index), state))
    }
}
