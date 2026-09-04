use std::collections::HashMap;

use laneflow_static_contract::{
    LaneEdgeOrdinal, ParkingFacilityOrdinal, ParkingSpaceOrdinal, VehicleProfileOrdinal,
};

use crate::migration_journal::{ParkingBindingDelta, VehicleDelta, WaitingMembershipReleaseDelta};
use crate::occupancy::{LeaderQueryHorizon, OccupancyIndex};
use crate::tables::{
    ConflictCapabilityError, VehicleSlot, occupancy_footprints_equal, occupancy_front_gap,
};
use crate::{ParkingError, RouteHandle, TrafficWorld, VehicleHandle, VehicleState, VehicleStatus};

/// 停车资源的精确目标。显式泊位与设施虚拟池是两个互不借用容量的 pool。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ParkingTarget {
    /// 一个具名、排他的显式泊位。
    ExplicitSpace(ParkingSpaceOrdinal),
    /// 一个设施拥有的稀疏虚拟容量池。
    VirtualPool(ParkingFacilityOrdinal),
}

/// 安装修订内、设施 owner-local 的虚拟入口 selector。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct VirtualEntryAnchorSelector(u32);

impl VirtualEntryAnchorSelector {
    #[must_use]
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }

    pub(crate) fn index(self) -> usize {
        usize::try_from(self.0).expect("virtual entry selector fits usize")
    }
}

/// 安装修订内、设施 owner-local 的虚拟出口 selector。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct VirtualExitAnchorSelector(u32);

impl VirtualExitAnchorSelector {
    #[must_use]
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }

    pub(crate) fn index(self) -> usize {
        usize::try_from(self.0).expect("virtual exit selector fits usize")
    }
}

/// 已提交 reservation 的完整 payload。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParkingReservation {
    target: ParkingTarget,
    route: RouteHandle,
    entry_route_occurrence: u32,
    virtual_entry_selector: Option<VirtualEntryAnchorSelector>,
}

impl ParkingReservation {
    pub(crate) const fn new(
        target: ParkingTarget,
        route: RouteHandle,
        entry_route_occurrence: u32,
        virtual_entry_selector: Option<VirtualEntryAnchorSelector>,
    ) -> Self {
        Self {
            target,
            route,
            entry_route_occurrence,
            virtual_entry_selector,
        }
    }

    #[must_use]
    pub const fn target(self) -> ParkingTarget {
        self.target
    }

    #[must_use]
    pub const fn route(self) -> RouteHandle {
        self.route
    }

    #[must_use]
    pub const fn entry_route_occurrence(self) -> u32 {
        self.entry_route_occurrence
    }

    #[must_use]
    pub const fn virtual_entry_selector(self) -> Option<VirtualEntryAnchorSelector> {
        self.virtual_entry_selector
    }
}

/// 车辆的只读停车 binding。唯一可修改权威由 `TrafficWorld` 内部 aggregate 持有。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParkingBinding {
    /// Active 车辆已消耗目标资源并驶向精确入口。
    Reserved(ParkingReservation),
    /// Parked 车辆占用精确目标。
    Occupied(ParkingTarget),
}

impl ParkingBinding {
    #[must_use]
    pub const fn target(self) -> ParkingTarget {
        match self {
            Self::Reserved(reservation) => reservation.target(),
            Self::Occupied(target) => target,
        }
    }
}

/// reserve 的 caller-owned typed payload。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReserveParkingTarget {
    ExplicitSpace {
        space: ParkingSpaceOrdinal,
        entry_route_occurrence: u32,
    },
    VirtualPool {
        facility: ParkingFacilityOrdinal,
        entry_anchor: VirtualEntryAnchorSelector,
        entry_route_occurrence: u32,
    },
}

impl ReserveParkingTarget {
    #[must_use]
    pub const fn target(self) -> ParkingTarget {
        match self {
            Self::ExplicitSpace { space, .. } => ParkingTarget::ExplicitSpace(space),
            Self::VirtualPool { facility, .. } => ParkingTarget::VirtualPool(facility),
        }
    }

    #[must_use]
    pub const fn entry_route_occurrence(self) -> u32 {
        match self {
            Self::ExplicitSpace {
                entry_route_occurrence,
                ..
            }
            | Self::VirtualPool {
                entry_route_occurrence,
                ..
            } => entry_route_occurrence,
        }
    }

    #[must_use]
    pub const fn virtual_entry_selector(self) -> Option<VirtualEntryAnchorSelector> {
        match self {
            Self::ExplicitSpace { .. } => None,
            Self::VirtualPool { entry_anchor, .. } => Some(entry_anchor),
        }
    }
}

/// leave 的 caller-owned typed payload。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LeaveParkingTarget {
    ExplicitSpace {
        space: ParkingSpaceOrdinal,
        route: RouteHandle,
        exit_route_occurrence: u32,
    },
    VirtualPool {
        facility: ParkingFacilityOrdinal,
        route: RouteHandle,
        exit_anchor: VirtualExitAnchorSelector,
        exit_route_occurrence: u32,
    },
}

impl LeaveParkingTarget {
    #[must_use]
    pub const fn target(self) -> ParkingTarget {
        match self {
            Self::ExplicitSpace { space, .. } => ParkingTarget::ExplicitSpace(space),
            Self::VirtualPool { facility, .. } => ParkingTarget::VirtualPool(facility),
        }
    }

    #[must_use]
    pub const fn route(self) -> RouteHandle {
        match self {
            Self::ExplicitSpace { route, .. } | Self::VirtualPool { route, .. } => route,
        }
    }

    #[must_use]
    pub const fn exit_route_occurrence(self) -> u32 {
        match self {
            Self::ExplicitSpace {
                exit_route_occurrence,
                ..
            }
            | Self::VirtualPool {
                exit_route_occurrence,
                ..
            } => exit_route_occurrence,
        }
    }

    #[must_use]
    pub const fn virtual_exit_selector(self) -> Option<VirtualExitAnchorSelector> {
        match self {
            Self::ExplicitSpace { .. } => None,
            Self::VirtualPool { exit_anchor, .. } => Some(exit_anchor),
        }
    }
}

/// rebind 的 caller-owned typed payload。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RebindParkingTarget {
    ExplicitSpace {
        space: ParkingSpaceOrdinal,
        new_route: RouteHandle,
        new_current_route_occurrence: u32,
        new_entry_route_occurrence: u32,
    },
    VirtualPool {
        facility: ParkingFacilityOrdinal,
        new_route: RouteHandle,
        new_current_route_occurrence: u32,
        new_entry_anchor: VirtualEntryAnchorSelector,
        new_entry_route_occurrence: u32,
    },
}

impl RebindParkingTarget {
    #[must_use]
    pub const fn target(self) -> ParkingTarget {
        match self {
            Self::ExplicitSpace { space, .. } => ParkingTarget::ExplicitSpace(space),
            Self::VirtualPool { facility, .. } => ParkingTarget::VirtualPool(facility),
        }
    }

    #[must_use]
    pub const fn new_route(self) -> RouteHandle {
        match self {
            Self::ExplicitSpace { new_route, .. } | Self::VirtualPool { new_route, .. } => {
                new_route
            }
        }
    }

    #[must_use]
    pub const fn new_current_route_occurrence(self) -> u32 {
        match self {
            Self::ExplicitSpace {
                new_current_route_occurrence,
                ..
            }
            | Self::VirtualPool {
                new_current_route_occurrence,
                ..
            } => new_current_route_occurrence,
        }
    }

    #[must_use]
    pub const fn new_entry_route_occurrence(self) -> u32 {
        match self {
            Self::ExplicitSpace {
                new_entry_route_occurrence,
                ..
            }
            | Self::VirtualPool {
                new_entry_route_occurrence,
                ..
            } => new_entry_route_occurrence,
        }
    }

    #[must_use]
    pub const fn virtual_entry_selector(self) -> Option<VirtualEntryAnchorSelector> {
        match self {
            Self::ExplicitSpace { .. } => None,
            Self::VirtualPool {
                new_entry_anchor, ..
            } => Some(new_entry_anchor),
        }
    }
}

/// 直接构造 `Parked + Occupied` 的输入。它保留 route cursor，但不建立 lane occupancy。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParkedVehicleSpawnInput {
    profile: VehicleProfileOrdinal,
    route: RouteHandle,
    route_occurrence: u32,
    progress_mm: u32,
}

impl ParkedVehicleSpawnInput {
    #[must_use]
    pub const fn new(
        profile: VehicleProfileOrdinal,
        route: RouteHandle,
        route_occurrence: u32,
        progress_mm: u32,
    ) -> Self {
        Self {
            profile,
            route,
            route_occurrence,
            progress_mm,
        }
    }

    #[must_use]
    pub const fn profile(self) -> VehicleProfileOrdinal {
        self.profile
    }

    #[must_use]
    pub const fn route(self) -> RouteHandle {
        self.route
    }

    #[must_use]
    pub const fn route_occurrence(self) -> u32 {
        self.route_occurrence
    }

    #[must_use]
    pub const fn progress_mm(self) -> u32 {
        self.progress_mm
    }
}

/// 支持窄幂等的命令结果。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParkingCommandOutcome<T> {
    Committed(T),
    NoChange(T),
}

impl<T> ParkingCommandOutcome<T> {
    #[must_use]
    pub const fn is_no_change(&self) -> bool {
        matches!(self, Self::NoChange(_))
    }

    #[must_use]
    pub fn into_record(self) -> T {
        match self {
            Self::Committed(record) | Self::NoChange(record) => record,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParkingReserveRecord {
    pub vehicle: VehicleHandle,
    pub target: ParkingTarget,
    pub route: RouteHandle,
    pub entry_route_occurrence: u32,
    pub virtual_entry_selector: Option<VirtualEntryAnchorSelector>,
    pub arrived: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParkingCancelRecord {
    pub vehicle: VehicleHandle,
    pub target: ParkingTarget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParkingParkRecord {
    pub vehicle: VehicleHandle,
    pub target: ParkingTarget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParkingLeaveRecord {
    pub vehicle: VehicleHandle,
    pub target: ParkingTarget,
    pub route: RouteHandle,
    pub exit_route_occurrence: u32,
    pub virtual_exit_selector: Option<VirtualExitAnchorSelector>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParkingRebindRecord {
    pub vehicle: VehicleHandle,
    pub target: ParkingTarget,
    pub old_route: RouteHandle,
    pub new_route: RouteHandle,
    pub old_current_route_occurrence: u32,
    pub new_current_route_occurrence: u32,
    pub new_entry_route_occurrence: u32,
    pub virtual_entry_selector: Option<VirtualEntryAnchorSelector>,
    pub arrived: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParkedVehicleSpawnRecord {
    pub vehicle: VehicleHandle,
    pub target: ParkingTarget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VehicleDespawnRecord {
    pub vehicle: VehicleHandle,
    pub status: VehicleStatus,
    pub parking_binding: Option<ParkingBinding>,
    pub waiting_release: Option<crate::WaitingMembershipReleaseRecord>,
    pub conflict_release: Option<crate::ConflictReservation>,
}

/// step 中首次提交 arrival 的稳定顺序 observation。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParkingArrivalObservation {
    pub vehicle: VehicleHandle,
    pub target: ParkingTarget,
}

/// 单个 pool 的守恒计数。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParkingPoolCounts {
    pub capacity: u64,
    pub reserved: u64,
    pub occupied: u64,
    pub vacant: u64,
}

impl ParkingPoolCounts {
    pub(crate) fn checked(capacity: u64, reserved: u64, occupied: u64) -> Option<Self> {
        let used = reserved.checked_add(occupied)?;
        let vacant = capacity.checked_sub(used)?;
        Some(Self {
            capacity,
            reserved,
            occupied,
            vacant,
        })
    }
}

/// 一个设施显式池、虚拟池与总量的分池查询。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParkingFacilityCounts {
    pub explicit: ParkingPoolCounts,
    pub virtual_pool: ParkingPoolCounts,
    pub total: ParkingPoolCounts,
}

/// 显式泊位的排他状态。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParkingSpaceState {
    Vacant,
    Reserved(VehicleHandle),
    Occupied(VehicleHandle),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct VirtualParkingState {
    pub(crate) reserved_count: u32,
    pub(crate) occupied_count: u32,
}

/// 停车运行时唯一修改权威。虚拟成员只按实际 binding `B` 稀疏保存。
#[derive(Clone, Debug)]
pub(crate) struct ParkingRuntimeState {
    explicit: Box<[ParkingSpaceState]>,
    virtual_pools: Box<[VirtualParkingState]>,
    bindings: HashMap<VehicleHandle, ParkingBinding>,
}

impl ParkingRuntimeState {
    pub(crate) fn new(explicit_space_count: usize, facility_count: usize) -> Self {
        Self {
            explicit: vec![ParkingSpaceState::Vacant; explicit_space_count].into_boxed_slice(),
            virtual_pools: vec![VirtualParkingState::default(); facility_count].into_boxed_slice(),
            bindings: HashMap::new(),
        }
    }

    pub(crate) fn try_new(explicit_space_count: usize, facility_count: usize) -> Result<Self, ()> {
        let mut explicit = Vec::new();
        explicit
            .try_reserve_exact(explicit_space_count)
            .map_err(|_| ())?;
        explicit.resize(explicit_space_count, ParkingSpaceState::Vacant);
        let mut virtual_pools = Vec::new();
        virtual_pools
            .try_reserve_exact(facility_count)
            .map_err(|_| ())?;
        virtual_pools.resize(facility_count, VirtualParkingState::default());
        Ok(Self {
            explicit: explicit.into_boxed_slice(),
            virtual_pools: virtual_pools.into_boxed_slice(),
            bindings: HashMap::new(),
        })
    }

    pub(crate) fn binding(&self, vehicle: VehicleHandle) -> Option<ParkingBinding> {
        self.bindings.get(&vehicle).copied()
    }

    pub(crate) fn explicit_state(&self, space: ParkingSpaceOrdinal) -> Option<ParkingSpaceState> {
        self.explicit.get(space.index()).copied()
    }

    pub(crate) fn virtual_state(
        &self,
        facility: ParkingFacilityOrdinal,
    ) -> Option<VirtualParkingState> {
        self.virtual_pools.get(facility.index()).copied()
    }

    pub(crate) fn try_reserve_binding(&mut self) -> Result<(), ()> {
        self.bindings.try_reserve(1).map_err(|_| ())
    }

    pub(crate) fn insert_reserved(
        &mut self,
        vehicle: VehicleHandle,
        reservation: ParkingReservation,
    ) {
        debug_assert!(!self.bindings.contains_key(&vehicle));
        match reservation.target() {
            ParkingTarget::ExplicitSpace(space) => {
                let state = self
                    .explicit
                    .get_mut(space.index())
                    .expect("validated parking space");
                debug_assert_eq!(*state, ParkingSpaceState::Vacant);
                *state = ParkingSpaceState::Reserved(vehicle);
            }
            ParkingTarget::VirtualPool(facility) => {
                let state = self
                    .virtual_pools
                    .get_mut(facility.index())
                    .expect("validated parking facility");
                state.reserved_count = state
                    .reserved_count
                    .checked_add(1)
                    .expect("validated virtual parking count");
            }
        }
        let replaced = self
            .bindings
            .insert(vehicle, ParkingBinding::Reserved(reservation));
        debug_assert!(replaced.is_none());
    }

    pub(crate) fn cancel_reserved(&mut self, vehicle: VehicleHandle) -> ParkingReservation {
        let Some(ParkingBinding::Reserved(reservation)) = self.bindings.remove(&vehicle) else {
            unreachable!("validated reservation remains present")
        };
        match reservation.target() {
            ParkingTarget::ExplicitSpace(space) => {
                let state = self
                    .explicit
                    .get_mut(space.index())
                    .expect("bound parking space exists");
                debug_assert_eq!(*state, ParkingSpaceState::Reserved(vehicle));
                *state = ParkingSpaceState::Vacant;
            }
            ParkingTarget::VirtualPool(facility) => {
                let state = self
                    .virtual_pools
                    .get_mut(facility.index())
                    .expect("bound parking facility exists");
                state.reserved_count = state
                    .reserved_count
                    .checked_sub(1)
                    .expect("bound virtual reservation counted");
            }
        }
        reservation
    }

    pub(crate) fn occupy_reserved(&mut self, vehicle: VehicleHandle) -> ParkingTarget {
        let Some(ParkingBinding::Reserved(reservation)) = self.bindings.get(&vehicle).copied()
        else {
            unreachable!("validated reservation remains present")
        };
        let target = reservation.target();
        match target {
            ParkingTarget::ExplicitSpace(space) => {
                let state = self
                    .explicit
                    .get_mut(space.index())
                    .expect("bound parking space exists");
                debug_assert_eq!(*state, ParkingSpaceState::Reserved(vehicle));
                *state = ParkingSpaceState::Occupied(vehicle);
            }
            ParkingTarget::VirtualPool(facility) => {
                let state = self
                    .virtual_pools
                    .get_mut(facility.index())
                    .expect("bound parking facility exists");
                state.reserved_count = state
                    .reserved_count
                    .checked_sub(1)
                    .expect("bound virtual reservation counted");
                state.occupied_count = state
                    .occupied_count
                    .checked_add(1)
                    .expect("validated virtual parking count");
            }
        }
        let old = self
            .bindings
            .insert(vehicle, ParkingBinding::Occupied(target));
        debug_assert!(matches!(old, Some(ParkingBinding::Reserved(_))));
        target
    }

    pub(crate) fn insert_occupied(&mut self, vehicle: VehicleHandle, target: ParkingTarget) {
        debug_assert!(!self.bindings.contains_key(&vehicle));
        match target {
            ParkingTarget::ExplicitSpace(space) => {
                let state = self
                    .explicit
                    .get_mut(space.index())
                    .expect("validated parking space");
                debug_assert_eq!(*state, ParkingSpaceState::Vacant);
                *state = ParkingSpaceState::Occupied(vehicle);
            }
            ParkingTarget::VirtualPool(facility) => {
                let state = self
                    .virtual_pools
                    .get_mut(facility.index())
                    .expect("validated parking facility");
                state.occupied_count = state
                    .occupied_count
                    .checked_add(1)
                    .expect("validated virtual parking count");
            }
        }
        let old = self
            .bindings
            .insert(vehicle, ParkingBinding::Occupied(target));
        debug_assert!(old.is_none());
    }

    pub(crate) fn release_occupied(&mut self, vehicle: VehicleHandle) -> ParkingTarget {
        let Some(ParkingBinding::Occupied(target)) = self.bindings.remove(&vehicle) else {
            unreachable!("validated occupied binding remains present")
        };
        match target {
            ParkingTarget::ExplicitSpace(space) => {
                let state = self
                    .explicit
                    .get_mut(space.index())
                    .expect("bound parking space exists");
                debug_assert_eq!(*state, ParkingSpaceState::Occupied(vehicle));
                *state = ParkingSpaceState::Vacant;
            }
            ParkingTarget::VirtualPool(facility) => {
                let state = self
                    .virtual_pools
                    .get_mut(facility.index())
                    .expect("bound parking facility exists");
                state.occupied_count = state
                    .occupied_count
                    .checked_sub(1)
                    .expect("bound virtual occupancy counted");
            }
        }
        target
    }

    pub(crate) fn replace_reservation(
        &mut self,
        vehicle: VehicleHandle,
        reservation: ParkingReservation,
    ) {
        let old = self
            .bindings
            .insert(vehicle, ParkingBinding::Reserved(reservation));
        debug_assert!(matches!(old, Some(ParkingBinding::Reserved(_))));
    }
}

#[derive(Clone, Copy)]
struct ResolvedAnchor {
    target: ParkingTarget,
    edge: LaneEdgeOrdinal,
    progress_mm: u32,
    route_occurrence: u32,
    entry_selector: Option<VirtualEntryAnchorSelector>,
}

fn parking_entry_forward_reachable(
    current_occurrence: u32,
    current_progress_mm: u32,
    current_carry_um: u16,
    entry_occurrence: u32,
    entry_progress_mm: u32,
) -> bool {
    entry_occurrence > current_occurrence
        || (entry_occurrence == current_occurrence
            && (entry_progress_mm > current_progress_mm
                || (entry_progress_mm == current_progress_mm && current_carry_um == 0)))
}

fn exact_parking_arrival(
    state: VehicleState,
    reservation: ParkingReservation,
    entry_progress_mm: u32,
) -> bool {
    state.status == VehicleStatus::Active
        && state.route == reservation.route()
        && state.route_edge_index == reservation.entry_route_occurrence()
        && state.progress_mm == entry_progress_mm
        && state.speed_mm_s == 0
        && state.carry_um == 0
}

fn map_waiting_parking_error(error: crate::waiting::WaitingBindingError) -> ParkingError {
    match error {
        crate::waiting::WaitingBindingError::VehicleTooLong => ParkingError::WaitingVehicleTooLong,
        crate::waiting::WaitingBindingError::StatefulManeuverInterior
        | crate::waiting::WaitingBindingError::AuthorityMismatch
        | crate::waiting::WaitingBindingError::ParkingConflict => {
            ParkingError::WaitingTraversalConflict
        }
        crate::waiting::WaitingBindingError::InvalidRoute => ParkingError::InvariantViolation,
    }
}

impl TrafficWorld {
    fn resolve_reserve_anchor(
        &self,
        input: ReserveParkingTarget,
    ) -> Result<ResolvedAnchor, ParkingError> {
        match input {
            ReserveParkingTarget::ExplicitSpace {
                space,
                entry_route_occurrence,
            } => {
                let view = self
                    .revision
                    .traffic()
                    .relations()
                    .parking_space(space)
                    .ok_or(ParkingError::UnknownSpace)?;
                let (edge, progress_mm) = view.entry();
                Ok(ResolvedAnchor {
                    target: ParkingTarget::ExplicitSpace(space),
                    edge,
                    progress_mm,
                    route_occurrence: entry_route_occurrence,
                    entry_selector: None,
                })
            }
            ReserveParkingTarget::VirtualPool {
                facility,
                entry_anchor,
                entry_route_occurrence,
            } => {
                let view = self
                    .revision
                    .traffic()
                    .relations()
                    .parking_facility(facility)
                    .ok_or(ParkingError::UnknownFacility)?;
                let anchor = view
                    .virtual_entries()
                    .get(entry_anchor.index())
                    .copied()
                    .ok_or(ParkingError::EntrySelectorNotOwned)?;
                Ok(ResolvedAnchor {
                    target: ParkingTarget::VirtualPool(facility),
                    edge: anchor.lane_edge(),
                    progress_mm: anchor.progress_mm(),
                    route_occurrence: entry_route_occurrence,
                    entry_selector: Some(entry_anchor),
                })
            }
        }
    }

    fn resolve_rebind_anchor(
        &self,
        input: RebindParkingTarget,
    ) -> Result<ResolvedAnchor, ParkingError> {
        match input {
            RebindParkingTarget::ExplicitSpace {
                space,
                new_entry_route_occurrence,
                ..
            } => self.resolve_reserve_anchor(ReserveParkingTarget::ExplicitSpace {
                space,
                entry_route_occurrence: new_entry_route_occurrence,
            }),
            RebindParkingTarget::VirtualPool {
                facility,
                new_entry_anchor,
                new_entry_route_occurrence,
                ..
            } => self.resolve_reserve_anchor(ReserveParkingTarget::VirtualPool {
                facility,
                entry_anchor: new_entry_anchor,
                entry_route_occurrence: new_entry_route_occurrence,
            }),
        }
    }

    fn resolve_leave_anchor(
        &self,
        input: LeaveParkingTarget,
    ) -> Result<(ParkingTarget, LaneEdgeOrdinal, u32), ParkingError> {
        match input {
            LeaveParkingTarget::ExplicitSpace { space, .. } => {
                let view = self
                    .revision
                    .traffic()
                    .relations()
                    .parking_space(space)
                    .ok_or(ParkingError::UnknownSpace)?;
                let (edge, progress_mm) = view.exit();
                Ok((ParkingTarget::ExplicitSpace(space), edge, progress_mm))
            }
            LeaveParkingTarget::VirtualPool {
                facility,
                exit_anchor,
                ..
            } => {
                let view = self
                    .revision
                    .traffic()
                    .relations()
                    .parking_facility(facility)
                    .ok_or(ParkingError::UnknownFacility)?;
                let anchor = view
                    .virtual_exits()
                    .get(exit_anchor.index())
                    .copied()
                    .ok_or(ParkingError::ExitSelectorNotOwned)?;
                Ok((
                    ParkingTarget::VirtualPool(facility),
                    anchor.lane_edge(),
                    anchor.progress_mm(),
                ))
            }
        }
    }

    fn validate_target_exists(&self, target: ParkingTarget) -> Result<(), ParkingError> {
        match target {
            ParkingTarget::ExplicitSpace(space) => self
                .revision
                .traffic()
                .relations()
                .parking_space(space)
                .map(|_| ())
                .ok_or(ParkingError::UnknownSpace),
            ParkingTarget::VirtualPool(facility) => self
                .revision
                .traffic()
                .relations()
                .parking_facility(facility)
                .map(|_| ())
                .ok_or(ParkingError::UnknownFacility),
        }
    }

    fn validate_anchor_on_route(
        &self,
        route: RouteHandle,
        occurrence: u32,
        anchor_edge: LaneEdgeOrdinal,
    ) -> Result<usize, ParkingError> {
        let edges = self.route_edges(route).ok_or(ParkingError::UnknownRoute)?;
        let index =
            usize::try_from(occurrence).map_err(|_| ParkingError::RouteOccurrenceOutOfRange)?;
        let edge = edges
            .get(index)
            .copied()
            .ok_or(ParkingError::RouteOccurrenceOutOfRange)?;
        if edge != anchor_edge {
            return Err(ParkingError::RouteOccurrenceAnchorMismatch);
        }
        Ok(index)
    }

    fn validate_forward_reachable(
        &self,
        state: VehicleState,
        entry_occurrence: u32,
        entry_progress_mm: u32,
    ) -> Result<(), ParkingError> {
        let reachable = parking_entry_forward_reachable(
            state.route_edge_index,
            state.progress_mm,
            state.carry_um,
            entry_occurrence,
            entry_progress_mm,
        );
        reachable
            .then_some(())
            .ok_or(ParkingError::EntryNotForwardReachable)
    }

    fn validate_available_target(&self, target: ParkingTarget) -> Result<(), ParkingError> {
        match target {
            ParkingTarget::ExplicitSpace(space) => match self
                .parking
                .explicit_state(space)
                .ok_or(ParkingError::UnknownSpace)?
            {
                ParkingSpaceState::Vacant => Ok(()),
                ParkingSpaceState::Reserved(_) | ParkingSpaceState::Occupied(_) => {
                    Err(ParkingError::TargetBoundByOther)
                }
            },
            ParkingTarget::VirtualPool(facility) => {
                let view = self
                    .revision
                    .traffic()
                    .relations()
                    .parking_facility(facility)
                    .ok_or(ParkingError::UnknownFacility)?;
                let state = self
                    .parking
                    .virtual_state(facility)
                    .ok_or(ParkingError::UnknownFacility)?;
                let used = state
                    .reserved_count
                    .checked_add(state.occupied_count)
                    .ok_or(ParkingError::InvariantViolation)?;
                if used >= view.virtual_capacity() {
                    return Err(ParkingError::VirtualCapacityExhausted);
                }
                Ok(())
            }
        }
    }

    pub(crate) fn resource_matches_binding(
        &self,
        vehicle: VehicleHandle,
        binding: ParkingBinding,
    ) -> bool {
        match binding {
            ParkingBinding::Reserved(reservation) => match reservation.target() {
                ParkingTarget::ExplicitSpace(space) => {
                    self.parking.explicit_state(space) == Some(ParkingSpaceState::Reserved(vehicle))
                }
                ParkingTarget::VirtualPool(facility) => self
                    .parking
                    .virtual_state(facility)
                    .is_some_and(|state| state.reserved_count > 0),
            },
            ParkingBinding::Occupied(target) => match target {
                ParkingTarget::ExplicitSpace(space) => {
                    self.parking.explicit_state(space) == Some(ParkingSpaceState::Occupied(vehicle))
                }
                ParkingTarget::VirtualPool(facility) => self
                    .parking
                    .virtual_state(facility)
                    .is_some_and(|state| state.occupied_count > 0),
            },
        }
    }

    pub(crate) fn reservation_anchor(
        &self,
        reservation: ParkingReservation,
    ) -> Option<(LaneEdgeOrdinal, u32)> {
        match reservation.target() {
            ParkingTarget::ExplicitSpace(space) => self
                .revision
                .traffic()
                .relations()
                .parking_space(space)
                .map(|view| view.entry()),
            ParkingTarget::VirtualPool(facility) => {
                let selector = reservation.virtual_entry_selector()?;
                let view = self
                    .revision
                    .traffic()
                    .relations()
                    .parking_facility(facility)?;
                let anchor = view.virtual_entries().get(selector.index())?;
                Some((anchor.lane_edge(), anchor.progress_mm()))
            }
        }
    }

    fn parking_binding_delta(&self, vehicle: VehicleHandle) -> ParkingBindingDelta {
        let binding = self.parking.binding(vehicle);
        let semantic_entry = match binding {
            Some(ParkingBinding::Reserved(reservation))
                if matches!(reservation.target(), ParkingTarget::VirtualPool(_)) =>
            {
                self.reservation_anchor(reservation)
            }
            _ => None,
        };
        ParkingBindingDelta::new(binding, semantic_entry)
    }

    fn record_parking_update(&mut self, vehicle: VehicleHandle, command_cursor: u64) {
        let state = *self
            .vehicle_state(vehicle)
            .expect("committed parking update keeps vehicle live");
        let delta = VehicleDelta::from_state(&state, self.compiled_route(state.route));
        let parking = self.parking_binding_delta(vehicle);
        if let Some(journal) = self.migration_journal.as_mut() {
            journal.record_vehicle_parking_updated(command_cursor, delta, parking);
        }
    }

    pub(crate) fn parking_arrived_for(
        &self,
        state: VehicleState,
        reservation: ParkingReservation,
    ) -> bool {
        let Some((_, entry_progress_mm)) = self.reservation_anchor(reservation) else {
            return false;
        };
        exact_parking_arrival(state, reservation, entry_progress_mm)
    }

    /// 是否已按 exact occurrence/progress/zero-motion 提交 arrival。
    #[must_use]
    pub fn parking_arrived(&self, vehicle: VehicleHandle, target: ParkingTarget) -> bool {
        let Some(state) = self.vehicle_state(vehicle).copied() else {
            return false;
        };
        let Some(ParkingBinding::Reserved(reservation)) = self.parking.binding(vehicle) else {
            return false;
        };
        reservation.target() == target && self.parking_arrived_for(state, reservation)
    }

    /// snapshot/cutover 共用的闭合状态矩阵与 reservation 语义复核。
    pub(crate) fn parking_state_valid(&self, vehicle: VehicleHandle) -> bool {
        let Some(state) = self.vehicle_state(vehicle).copied() else {
            return false;
        };
        let binding = self.parking.binding(vehicle);
        if !matches!(
            (state.status, binding),
            (
                VehicleStatus::Active,
                None | Some(ParkingBinding::Reserved(_))
            ) | (VehicleStatus::Parked, Some(ParkingBinding::Occupied(_)))
                | (VehicleStatus::Completed, None)
        ) {
            return false;
        }
        let Some(binding) = binding else {
            return true;
        };
        if !self.resource_matches_binding(vehicle, binding) {
            return false;
        }
        let ParkingBinding::Reserved(reservation) = binding else {
            return true;
        };
        if reservation.route() != state.route {
            return false;
        }
        let Some((edge, progress_mm)) = self.reservation_anchor(reservation) else {
            return false;
        };
        if self
            .validate_anchor_on_route(
                reservation.route(),
                reservation.entry_route_occurrence(),
                edge,
            )
            .is_err()
        {
            return false;
        }
        if self
            .validate_waiting_parking_anchor(
                reservation.route(),
                reservation.entry_route_occurrence(),
            )
            .is_err()
        {
            return false;
        }
        self.validate_forward_reachable(state, reservation.entry_route_occurrence(), progress_mm)
            .is_ok()
    }

    fn checked_parking_observation_commit(
        &self,
    ) -> Result<(u64, crate::ObservationStateSequence), ParkingError> {
        let command_cursor = self.checked_parking_command()?;
        let observation_state_sequence = self
            .observation_state_sequence
            .checked_next()
            .ok_or(ParkingError::ObservationStateSequenceExhausted)?;
        Ok((command_cursor, observation_state_sequence))
    }

    fn checked_parking_command(&self) -> Result<u64, ParkingError> {
        self.command_cursor
            .checked_add(1)
            .ok_or(ParkingError::CommandCursorExhausted)
    }

    fn route_ref_increment(&self, route: RouteHandle) -> Result<u32, ParkingError> {
        let index = usize::try_from(route.index()).map_err(|_| ParkingError::UnknownRoute)?;
        let slot = self.routes.get(index).ok_or(ParkingError::UnknownRoute)?;
        if slot.generation != route.generation() || slot.compiled.is_none() {
            return Err(ParkingError::UnknownRoute);
        }
        slot.live_vehicles
            .checked_add(1)
            .ok_or(ParkingError::RouteReferenceCapacityExceeded)
    }

    fn commit_route_ref_increment(&mut self, route: RouteHandle, value: u32) {
        let index = usize::try_from(route.index()).expect("validated route index");
        self.routes[index].live_vehicles = value;
    }

    /// 原子预留显式泊位或虚拟池容量。
    pub fn reserve_parking(
        &mut self,
        vehicle: VehicleHandle,
        input: ReserveParkingTarget,
    ) -> Result<ParkingCommandOutcome<ParkingReserveRecord>, ParkingError> {
        let state = self
            .vehicle_state(vehicle)
            .copied()
            .ok_or(ParkingError::StaleVehicle)?;
        if state.status != VehicleStatus::Active {
            return Err(ParkingError::InvalidVehicleStatus);
        }
        let anchor = self.resolve_reserve_anchor(input)?;
        let occurrence =
            self.validate_anchor_on_route(state.route, anchor.route_occurrence, anchor.edge)?;
        if self.route_suffix_denied(state.route, state.class, occurrence) {
            return Err(ParkingError::AccessDenied);
        }
        self.validate_forward_reachable(state, anchor.route_occurrence, anchor.progress_mm)?;
        self.validate_waiting_parking_anchor(state.route, anchor.route_occurrence)
            .map_err(|error| match error {
                crate::waiting::WaitingBindingError::ParkingConflict => {
                    ParkingError::WaitingTraversalConflict
                }
                _ => ParkingError::InvariantViolation,
            })?;
        let reservation = ParkingReservation::new(
            anchor.target,
            state.route,
            anchor.route_occurrence,
            anchor.entry_selector,
        );
        let record = ParkingReserveRecord {
            vehicle,
            target: anchor.target,
            route: state.route,
            entry_route_occurrence: anchor.route_occurrence,
            virtual_entry_selector: anchor.entry_selector,
            arrived: self.parking_arrived_for(state, reservation),
        };
        if let Some(binding) = self.parking.binding(vehicle) {
            if binding == ParkingBinding::Reserved(reservation)
                && self.resource_matches_binding(vehicle, binding)
            {
                let cursor = self.checked_parking_command()?;
                self.command_cursor = cursor;
                return Ok(ParkingCommandOutcome::NoChange(record));
            }
            return Err(ParkingError::VehicleAlreadyBound);
        }
        self.validate_available_target(anchor.target)?;
        let command_cursor = self.checked_parking_command()?;
        self.parking
            .try_reserve_binding()
            .map_err(|()| ParkingError::AllocationFailed)?;

        self.parking.insert_reserved(vehicle, reservation);
        self.command_cursor = command_cursor;
        self.record_parking_update(vehicle, command_cursor);
        Ok(ParkingCommandOutcome::Committed(record))
    }

    /// 取消 exact reservation；重复取消是 `NotReserved`，不是 no-op。
    pub fn cancel_parking(
        &mut self,
        vehicle: VehicleHandle,
        target: ParkingTarget,
    ) -> Result<ParkingCancelRecord, ParkingError> {
        self.validate_target_exists(target)?;
        let state = self
            .vehicle_state(vehicle)
            .copied()
            .ok_or(ParkingError::StaleVehicle)?;
        if state.status != VehicleStatus::Active {
            return Err(ParkingError::InvalidVehicleStatus);
        }
        let binding = self
            .parking
            .binding(vehicle)
            .ok_or(ParkingError::NotReserved)?;
        let ParkingBinding::Reserved(reservation) = binding else {
            return Err(ParkingError::NotReserved);
        };
        if reservation.target() != target {
            return Err(ParkingError::NotReserved);
        }
        if reservation.route() != state.route || !self.resource_matches_binding(vehicle, binding) {
            return Err(ParkingError::InvariantViolation);
        }
        let command_cursor = self.checked_parking_command()?;
        self.parking.cancel_reserved(vehicle);
        self.command_cursor = command_cursor;
        self.record_parking_update(vehicle, command_cursor);
        Ok(ParkingCancelRecord { vehicle, target })
    }

    /// 把 exact arrived reservation 原子提交为 `Parked + Occupied`。
    pub fn park_vehicle(
        &mut self,
        vehicle: VehicleHandle,
        target: ParkingTarget,
    ) -> Result<ParkingCommandOutcome<ParkingParkRecord>, ParkingError> {
        self.validate_target_exists(target)?;
        let state = self
            .vehicle_state(vehicle)
            .copied()
            .ok_or(ParkingError::StaleVehicle)?;
        let record = ParkingParkRecord { vehicle, target };
        match self.parking.binding(vehicle) {
            Some(binding @ ParkingBinding::Occupied(current))
                if current == target
                    && state.status == VehicleStatus::Parked
                    && self.resource_matches_binding(vehicle, binding) =>
            {
                let cursor = self.checked_parking_command()?;
                self.command_cursor = cursor;
                return Ok(ParkingCommandOutcome::NoChange(record));
            }
            Some(ParkingBinding::Reserved(reservation)) if reservation.target() == target => {
                if state.status != VehicleStatus::Active {
                    return Err(ParkingError::InvariantViolation);
                }
                if self.vehicle_has_conflict_authority(vehicle) {
                    return Err(ParkingError::ConflictTraversalActive);
                }
                if reservation.route() != state.route
                    || !self
                        .resource_matches_binding(vehicle, ParkingBinding::Reserved(reservation))
                {
                    return Err(ParkingError::InvariantViolation);
                }
                if !self.parking_arrived_for(state, reservation) {
                    return Err(ParkingError::NotArrived);
                }
                if state.maneuver_traversal.is_some() || state.waiting_membership.is_some() {
                    return Err(ParkingError::WaitingTraversalConflict);
                }
            }
            Some(_) | None => return Err(ParkingError::NotReserved),
        }
        let (command_cursor, sequence) = self.checked_parking_observation_commit()?;
        self.parking.occupy_reserved(vehicle);
        let index = usize::try_from(vehicle.index()).expect("validated vehicle index");
        let state = self.vehicles[index]
            .state
            .as_mut()
            .expect("validated live vehicle");
        state.status = VehicleStatus::Parked;
        state.speed_mm_s = 0;
        state.carry_um = 0;
        self.rebuild_active_order();
        self.command_cursor = command_cursor;
        self.observation_state_sequence = sequence;
        self.record_parking_update(vehicle, command_cursor);
        Ok(ParkingCommandOutcome::Committed(record))
    }

    fn validate_leave_followers(
        &self,
        candidate: VehicleState,
        occupancy: &OccupancyIndex,
    ) -> Result<(), ParkingError> {
        let lengths = self.revision.traffic().lane_lengths_millimetres();
        let candidate_edges = self
            .route_edges(candidate.route)
            .ok_or(ParkingError::UnknownRoute)?;
        let candidate_index = usize::try_from(candidate.route_edge_index)
            .map_err(|_| ParkingError::RouteOccurrenceOutOfRange)?;
        let delta_s = self.config.fixed_delta_time_ms() as f32 / 1_000.0;
        for handle in self.active_order.iter().copied() {
            if handle == candidate.handle {
                continue;
            }
            let Some(follower) = self.vehicle_state(handle).copied() else {
                continue;
            };
            if follower.status != VehicleStatus::Active || follower.speed_mm_s == 0 {
                continue;
            }
            let follower_edges = self
                .route_edges(follower.route)
                .ok_or(ParkingError::InvariantViolation)?;
            let follower_index = usize::try_from(follower.route_edge_index)
                .map_err(|_| ParkingError::InvariantViolation)?;
            let Some(candidate_gap) = occupancy_front_gap(
                lengths,
                follower_edges,
                follower_index,
                follower.progress_mm,
                candidate_edges,
                candidate_index,
                candidate.progress_mm,
                candidate.length_mm,
            ) else {
                continue;
            };
            let candidate_gap_limit = u32::try_from(candidate_gap.max(0)).unwrap_or(u32::MAX);
            if occupancy
                .leader_gap(
                    follower.handle,
                    follower_edges,
                    follower_index,
                    follower.progress_mm,
                    lengths,
                    LeaderQueryHorizon::new(candidate_gap_limit, u32::MAX),
                )
                .is_some()
            {
                continue;
            }
            let profile = self
                .revision
                .traffic()
                .relations()
                .vehicle_profile(follower.profile)
                .ok_or(ParkingError::InvariantViolation)?;
            let v = follower.speed_mm_s as f32 / 1_000.0;
            let emergency = profile.emergency_decel();
            let gap_mm = u32::try_from(candidate_gap.max(0)).unwrap_or(u32::MAX);
            let gap_m = gap_mm as f32 / 1_000.0;
            let preserved_gap_mm = gap_mm.min(profile.min_gap_mm());
            let raw_available_gap_mm = gap_mm.saturating_sub(preserved_gap_mm);
            let available_gap_mm = if raw_available_gap_mm <= 1 {
                0
            } else {
                raw_available_gap_mm
            };
            if ![v, emergency, delta_s, gap_m]
                .into_iter()
                .all(f32::is_finite)
                || emergency <= 0.0
                || delta_s <= 0.0
            {
                return Err(ParkingError::LeaveUnsafeFollower { follower: handle });
            }
            let u_min = (v - emergency * delta_s).max(0.0);
            let safe_envelope = 0.5 * (v + u_min) * delta_s + u_min * u_min / (2.0 * emergency);
            let emergency_min_travel = if v <= emergency * delta_s {
                v * v / (2.0 * emergency)
            } else {
                v * delta_s - 0.5 * emergency * delta_s * delta_s
            };
            let available_m = available_gap_mm as f32 / 1_000.0;
            if !safe_envelope.is_finite()
                || !emergency_min_travel.is_finite()
                || safe_envelope > gap_m
                || emergency_min_travel > available_m
            {
                return Err(ParkingError::LeaveUnsafeFollower { follower: handle });
            }
        }
        Ok(())
    }

    /// 从 `Parked + Occupied` 安全插入 exact exit anchor，并原子释放资源。
    pub fn leave_parking(
        &mut self,
        vehicle: VehicleHandle,
        input: LeaveParkingTarget,
    ) -> Result<ParkingLeaveRecord, ParkingError> {
        let state = self
            .vehicle_state(vehicle)
            .copied()
            .ok_or(ParkingError::StaleVehicle)?;
        if state.status != VehicleStatus::Parked {
            return Err(ParkingError::InvalidVehicleStatus);
        }
        let (target, exit_edge, exit_progress_mm) = self.resolve_leave_anchor(input)?;
        let binding = self
            .parking
            .binding(vehicle)
            .ok_or(ParkingError::NotOccupied)?;
        if binding != ParkingBinding::Occupied(target) {
            return Err(ParkingError::NotOccupied);
        }
        if !self.resource_matches_binding(vehicle, binding) {
            return Err(ParkingError::InvariantViolation);
        }
        let route = input.route();
        let occurrence =
            self.validate_anchor_on_route(route, input.exit_route_occurrence(), exit_edge)?;
        if self.route_suffix_denied(route, state.class, occurrence) {
            return Err(ParkingError::AccessDenied);
        }
        let candidate = VehicleState {
            route,
            route_edge_index: input.exit_route_occurrence(),
            progress_mm: exit_progress_mm,
            carry_um: 0,
            speed_mm_s: 0,
            status: VehicleStatus::Active,
            ..state
        };
        let traversal = self
            .validate_waiting_bootstrap(route, occurrence, state.length_mm)
            .map_err(map_waiting_parking_error)?;
        let candidate = VehicleState {
            maneuver_traversal: traversal,
            waiting_membership: None,
            ..candidate
        };
        if let Some(blocker) =
            self.overlap_blocker(route, occurrence, exit_progress_mm, state.length_mm)
        {
            return Err(ParkingError::LeavePhysicalOverlap { blocker });
        }
        let occupancy = self
            .build_occupancy_index_for(self.revision.as_ref(), &[])
            .map_err(|error| match error {
                crate::StepError::OccupancyAllocFailed => ParkingError::AllocationFailed,
                _ => ParkingError::InvariantViolation,
            })?;
        self.validate_leave_followers(candidate, &occupancy)?;
        match self.check_active_conflict_capability(
            route,
            occurrence,
            exit_progress_mm,
            0,
            state.length_mm,
        ) {
            Ok(()) => {}
            Err(ConflictCapabilityError::InvalidCursor) => {
                return Err(ParkingError::InvariantViolation);
            }
            Err(ConflictCapabilityError::AuthorityRequired) => {
                return Err(ParkingError::ConflictAuthorityRequired);
            }
        }
        let new_route_ref = (route != state.route)
            .then(|| self.route_ref_increment(route))
            .transpose()?;
        let (command_cursor, sequence) = self.checked_parking_observation_commit()?;

        self.parking.release_occupied(vehicle);
        if let Some(new_route_ref) = new_route_ref {
            self.release_route_ref(state.route);
            self.commit_route_ref_increment(route, new_route_ref);
        }
        let index = usize::try_from(vehicle.index()).expect("validated vehicle index");
        self.vehicles[index].state = Some(candidate);
        self.rebuild_active_order();
        self.command_cursor = command_cursor;
        self.observation_state_sequence = sequence;
        self.record_parking_update(vehicle, command_cursor);
        Ok(ParkingLeaveRecord {
            vehicle,
            target,
            route,
            exit_route_occurrence: input.exit_route_occurrence(),
            virtual_exit_selector: input.virtual_exit_selector(),
        })
    }

    /// 在保持完整物理 footprint 的前提下更换 Reserved route/entry payload。
    pub fn rebind_parking_route(
        &mut self,
        vehicle: VehicleHandle,
        input: RebindParkingTarget,
    ) -> Result<ParkingCommandOutcome<ParkingRebindRecord>, ParkingError> {
        let state = self
            .vehicle_state(vehicle)
            .copied()
            .ok_or(ParkingError::StaleVehicle)?;
        if state.status != VehicleStatus::Active {
            return Err(ParkingError::InvalidVehicleStatus);
        }
        if self.vehicle_has_conflict_authority(vehicle) {
            return Err(ParkingError::ConflictTraversalActive);
        }
        let anchor = self.resolve_rebind_anchor(input)?;
        let binding = self
            .parking
            .binding(vehicle)
            .ok_or(ParkingError::NotReserved)?;
        let ParkingBinding::Reserved(old_reservation) = binding else {
            return Err(ParkingError::NotReserved);
        };
        if old_reservation.target() != anchor.target {
            return Err(ParkingError::NotReserved);
        }
        if old_reservation.route() != state.route
            || !self.resource_matches_binding(vehicle, binding)
        {
            return Err(ParkingError::InvariantViolation);
        }
        let new_route = input.new_route();
        let new_edges = self
            .route_edges(new_route)
            .ok_or(ParkingError::UnknownRoute)?;
        let new_current_index = usize::try_from(input.new_current_route_occurrence())
            .map_err(|_| ParkingError::RouteOccurrenceOutOfRange)?;
        let current_edge = self
            .route_edges(state.route)
            .and_then(|edges| edges.get(usize::try_from(state.route_edge_index).ok()?))
            .copied()
            .ok_or(ParkingError::InvariantViolation)?;
        if new_edges.get(new_current_index).copied() != Some(current_edge) {
            return Err(ParkingError::RebindCurrentOccurrenceMismatch);
        }
        let entry_index =
            self.validate_anchor_on_route(new_route, anchor.route_occurrence, anchor.edge)?;
        if self.route_suffix_denied(new_route, state.class, new_current_index) {
            return Err(ParkingError::AccessDenied);
        }
        let old_edges = self
            .route_edges(state.route)
            .ok_or(ParkingError::InvariantViolation)?;
        let lengths = self.revision.traffic().lane_lengths_millimetres();
        let footprints_equal = occupancy_footprints_equal(
            lengths,
            old_edges,
            usize::try_from(state.route_edge_index)
                .map_err(|_| ParkingError::InvariantViolation)?,
            state.progress_mm,
            state.length_mm,
            new_edges,
            new_current_index,
            state.progress_mm,
            state.length_mm,
        )
        .map_err(|()| ParkingError::AllocationFailed)?;
        if !footprints_equal {
            return Err(ParkingError::RebindBodyFootprintMismatch);
        }
        let (maneuver_traversal, waiting_membership) = self
            .rebind_waiting_authority(state, new_route, new_current_index)
            .map_err(map_waiting_parking_error)?;
        let candidate = VehicleState {
            route: new_route,
            route_edge_index: input.new_current_route_occurrence(),
            maneuver_traversal,
            waiting_membership,
            ..state
        };
        self.validate_forward_reachable(candidate, anchor.route_occurrence, anchor.progress_mm)?;
        self.validate_waiting_parking_anchor(new_route, anchor.route_occurrence)
            .map_err(map_waiting_parking_error)?;
        match self.check_active_conflict_capability(
            new_route,
            new_current_index,
            state.progress_mm,
            state.carry_um,
            state.length_mm,
        ) {
            Ok(()) => {}
            Err(ConflictCapabilityError::InvalidCursor) => {
                return Err(ParkingError::InvariantViolation);
            }
            Err(ConflictCapabilityError::AuthorityRequired) => {
                return Err(ParkingError::ConflictAuthorityRequired);
            }
        }
        let reservation = ParkingReservation::new(
            anchor.target,
            new_route,
            anchor.route_occurrence,
            anchor.entry_selector,
        );
        let arrived = self.parking_arrived_for(candidate, reservation);
        let record = ParkingRebindRecord {
            vehicle,
            target: anchor.target,
            old_route: state.route,
            new_route,
            old_current_route_occurrence: state.route_edge_index,
            new_current_route_occurrence: input.new_current_route_occurrence(),
            new_entry_route_occurrence: anchor.route_occurrence,
            virtual_entry_selector: anchor.entry_selector,
            arrived,
        };
        if state.route == new_route
            && state.route_edge_index == input.new_current_route_occurrence()
            && old_reservation == reservation
        {
            let cursor = self.checked_parking_command()?;
            self.command_cursor = cursor;
            return Ok(ParkingCommandOutcome::NoChange(record));
        }
        let new_route_ref = (new_route != state.route)
            .then(|| self.route_ref_increment(new_route))
            .transpose()?;
        let command_cursor = self.checked_parking_command()?;

        if let Some(new_route_ref) = new_route_ref {
            self.release_route_ref(state.route);
            self.commit_route_ref_increment(new_route, new_route_ref);
        }
        let index = usize::try_from(vehicle.index()).expect("validated vehicle index");
        self.vehicles[index].state = Some(candidate);
        self.parking.replace_reservation(vehicle, reservation);
        self.rebuild_waiting_member_rows();
        self.command_cursor = command_cursor;
        self.record_parking_update(vehicle, command_cursor);
        let _ = entry_index;
        Ok(ParkingCommandOutcome::Committed(record))
    }

    /// 直接构造 `Parked + Occupied`；不伪造 reservation 或入口 arrival。
    pub fn spawn_parked_vehicle(
        &mut self,
        input: ParkedVehicleSpawnInput,
        target: ParkingTarget,
    ) -> Result<ParkedVehicleSpawnRecord, ParkingError> {
        self.validate_target_exists(target)?;
        self.validate_available_target(target)?;
        let live = u32::try_from(self.live_order.len())
            .map_err(|_| ParkingError::VehicleCapacityExceeded)?;
        if live >= self.config.vehicle_capacity() {
            return Err(ParkingError::VehicleCapacityExceeded);
        }
        let profile = self
            .revision
            .traffic()
            .relations()
            .vehicle_profile(input.profile())
            .ok_or(ParkingError::UnknownProfile)?;
        let route_edges = self
            .route_edges(input.route())
            .ok_or(ParkingError::UnknownRoute)?;
        let occurrence = usize::try_from(input.route_occurrence())
            .map_err(|_| ParkingError::RouteOccurrenceOutOfRange)?;
        let edge = route_edges
            .get(occurrence)
            .copied()
            .ok_or(ParkingError::RouteOccurrenceOutOfRange)?;
        let edge_length = self.revision.traffic().lane_lengths_millimetres()[edge.index()];
        if input.progress_mm() > edge_length {
            return Err(ParkingError::InvalidProgress);
        }
        if self.route_suffix_denied(input.route(), profile.class(), occurrence) {
            return Err(ParkingError::AccessDenied);
        }
        // Parked retained cursor 不是道路 arrival；真正离场时再验证 Active 绑定。
        let route_ref = self.route_ref_increment(input.route())?;
        let command_cursor = self.checked_parking_command()?;
        self.parking
            .try_reserve_binding()
            .map_err(|()| ParkingError::AllocationFailed)?;

        let slot_index = self.free_vehicles.pop().unwrap_or(self.vehicles.len());
        let generation = self
            .vehicles
            .get(slot_index)
            .map_or(0, |slot| slot.generation);
        let vehicle = VehicleHandle::new(
            u32::try_from(slot_index).expect("vehicle index fits u32"),
            generation,
        );
        let state = VehicleState {
            handle: vehicle,
            profile: input.profile(),
            class: profile.class(),
            route: input.route(),
            route_edge_index: input.route_occurrence(),
            progress_mm: input.progress_mm(),
            carry_um: 0,
            speed_mm_s: 0,
            length_mm: profile.length_mm(),
            status: VehicleStatus::Parked,
            maneuver_traversal: None,
            waiting_membership: None,
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
        self.live_order.push(vehicle);
        self.commit_route_ref_increment(input.route(), route_ref);
        self.parking.insert_occupied(vehicle, target);
        self.command_cursor = command_cursor;
        let delta = VehicleDelta::from_state(&state, self.compiled_route(state.route));
        let parking = self.parking_binding_delta(vehicle);
        if let Some(journal) = self.migration_journal.as_mut() {
            journal.record_vehicle_parking_spawned(command_cursor, delta, parking);
        }
        Ok(ParkedVehicleSpawnRecord { vehicle, target })
    }

    /// 真正移除任意 live lifecycle 状态，同时释放 route 与可选停车 binding。
    pub fn despawn_vehicle(
        &mut self,
        vehicle: VehicleHandle,
    ) -> Result<VehicleDespawnRecord, ParkingError> {
        let state = self
            .vehicle_state(vehicle)
            .copied()
            .ok_or(ParkingError::StaleVehicle)?;
        let binding = self.parking.binding(vehicle);
        if !self.waiting_state_valid() || !self.conflict_state_valid() {
            return Err(ParkingError::InvariantViolation);
        }
        let valid = matches!(
            (state.status, binding),
            (
                VehicleStatus::Active,
                None | Some(ParkingBinding::Reserved(_))
            ) | (VehicleStatus::Parked, Some(ParkingBinding::Occupied(_)))
                | (VehicleStatus::Completed, None)
        );
        if !valid || binding.is_some_and(|value| !self.resource_matches_binding(vehicle, value)) {
            return Err(ParkingError::InvariantViolation);
        }
        let conflict_release = self.conflict_reservation(vehicle);
        let order_index = self
            .live_order
            .iter()
            .position(|handle| *handle == vehicle)
            .ok_or(ParkingError::InvariantViolation)?;
        let (command_cursor, sequence) = match state.status {
            VehicleStatus::Active => {
                let (command_cursor, sequence) = self.checked_parking_observation_commit()?;
                (command_cursor, Some(sequence))
            }
            VehicleStatus::Parked | VehicleStatus::Completed => {
                (self.checked_parking_command()?, None)
            }
        };
        let waiting_release_delta =
            WaitingMembershipReleaseDelta::from_state(&state, self.compiled_route(state.route));
        let record = VehicleDespawnRecord {
            vehicle,
            status: state.status,
            parking_binding: binding,
            waiting_release: state.waiting_membership.map(|membership| {
                let traversal = state
                    .maneuver_traversal
                    .expect("validated Waiting member has traversal");
                crate::WaitingMembershipReleaseRecord {
                    waiting_zone: membership.waiting_zone,
                    route_anchor: crate::WaitingRouteAnchor {
                        route: state.route,
                        maneuver_occurrence_index: traversal.maneuver_occurrence_index,
                        hop: membership.release_hop,
                    },
                    admission_sequence: membership.admission_sequence,
                }
            }),
            conflict_release,
        };

        match binding {
            Some(ParkingBinding::Reserved(_)) => {
                self.parking.cancel_reserved(vehicle);
            }
            Some(ParkingBinding::Occupied(_)) => {
                self.parking.release_occupied(vehicle);
            }
            None => {}
        }
        self.conflict_arbiter.release_vehicle(vehicle, self.time_ms);
        self.clear_conflict_eligibility(vehicle);
        self.release_route_ref(state.route);
        if let Some(membership) = state.waiting_membership {
            self.unlink_waiting_member(vehicle, membership);
        }
        self.live_order.remove(order_index);
        let slot_index = usize::try_from(vehicle.index()).expect("validated vehicle index");
        let slot = &mut self.vehicles[slot_index];
        slot.state = None;
        let mut recyclable = false;
        if let Some(next_generation) = slot.generation.checked_add(1) {
            slot.generation = next_generation;
            self.free_vehicles.push(slot_index);
            recyclable = true;
        }
        let generation_after = slot.generation;
        self.rebuild_active_order();
        self.rebuild_waiting_member_rows();
        self.command_cursor = command_cursor;
        if let Some(sequence) = sequence {
            self.observation_state_sequence = sequence;
        }
        if let Some(journal) = self.migration_journal.as_mut() {
            journal.record_vehicle_despawned(
                command_cursor,
                vehicle,
                u32::try_from(order_index).expect("live order index fits u32"),
                recyclable,
                generation_after,
                waiting_release_delta,
            );
        }
        Ok(record)
    }
}

#[cfg(test)]
mod tests {
    use laneflow_static_contract::{ParticipantClassOrdinal, VehicleProfileOrdinal};

    use super::*;

    fn arrival_fixture() -> (VehicleState, ParkingReservation) {
        let vehicle = VehicleHandle::new(3, 7);
        let route = RouteHandle::new(5, 11);
        (
            VehicleState {
                handle: vehicle,
                profile: VehicleProfileOrdinal::from_raw(2),
                class: ParticipantClassOrdinal::from_raw(4),
                route,
                route_edge_index: 6,
                progress_mm: 20_000,
                carry_um: 0,
                speed_mm_s: 0,
                length_mm: 4_500,
                status: VehicleStatus::Active,
                maneuver_traversal: None,
                waiting_membership: None,
            },
            ParkingReservation::new(
                ParkingTarget::VirtualPool(ParkingFacilityOrdinal::from_raw(1)),
                route,
                6,
                Some(VirtualEntryAnchorSelector::from_raw(2)),
            ),
        )
    }

    #[test]
    fn arrival_requires_every_exact_committed_field() {
        let (base, reservation) = arrival_fixture();
        assert!(exact_parking_arrival(base, reservation, 20_000));

        let counterexamples = [
            VehicleState {
                route: RouteHandle::new(6, 11),
                ..base
            },
            VehicleState {
                route_edge_index: 5,
                ..base
            },
            VehicleState {
                progress_mm: 20_001,
                ..base
            },
            VehicleState {
                speed_mm_s: 1,
                ..base
            },
            VehicleState {
                carry_um: 1,
                ..base
            },
            VehicleState {
                status: VehicleStatus::Parked,
                ..base
            },
        ];
        for state in counterexamples {
            assert!(!exact_parking_arrival(state, reservation, 20_000));
        }
        assert_ne!(
            reservation.target(),
            ParkingTarget::ExplicitSpace(ParkingSpaceOrdinal::from_raw(0))
        );
    }

    #[test]
    fn forward_reachability_closes_the_sub_millimetre_boundary() {
        assert!(parking_entry_forward_reachable(3, 20_000, 0, 3, 20_000));
        assert!(!parking_entry_forward_reachable(3, 20_000, 1, 3, 20_000));
        assert!(parking_entry_forward_reachable(3, 20_000, 999, 3, 20_001));
        assert!(parking_entry_forward_reachable(3, 20_000, 999, 4, 0));
        assert!(!parking_entry_forward_reachable(3, 20_000, 0, 2, 99_999));
        assert!(!parking_entry_forward_reachable(3, 20_000, 0, 3, 19_999));
    }

    #[test]
    fn declared_virtual_capacity_is_not_a_runtime_storage_axis() {
        let state = ParkingRuntimeState::new(3, 2);
        assert_eq!(state.explicit.len(), 3);
        assert_eq!(state.virtual_pools.len(), 2);
        assert!(state.bindings.is_empty());
    }
}
