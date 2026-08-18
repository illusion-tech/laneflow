use crate::{ManeuverGateHandle, ParkingSpaceHandle, RouteHandle, VehicleHandle};

/// 每车固定步进私有调用链使用的紧凑错误。
///
/// 该类型只保存构造公开诊断所需的有类型句柄和数值。external ID 与 `String` 仅在
/// `CoreWorld` 的公开步进边界展开。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum TickInvariantError {
    NonFiniteParkingComputation {
        stage: &'static str,
        vehicle: VehicleHandle,
        space: ParkingSpaceHandle,
        value: f64,
    },
    NonFiniteLeaderComputation {
        vehicle: VehicleHandle,
        stage: &'static str,
        value: f64,
    },
    NonFiniteLongitudinalComputation {
        vehicle: VehicleHandle,
        stage: &'static str,
        value: f64,
    },
    NonFiniteSpeedLimitComputation {
        vehicle: VehicleHandle,
        stage: &'static str,
        value: f64,
    },
    SpeedLimitTraversalInvariant {
        vehicle: VehicleHandle,
        route: RouteHandle,
        from_route_edge_index: usize,
        to_route_edge_index: usize,
        final_speed: f64,
        target_limit: f64,
    },
    NonFiniteSignalStopComputation {
        vehicle: VehicleHandle,
        stage: &'static str,
        value: f64,
    },
    NonFiniteRouteTravel {
        vehicle: VehicleHandle,
        speed: f64,
        delta_time_ms: u64,
    },
    SignalTraversalDeniedInvariant {
        vehicle: VehicleHandle,
        route: RouteHandle,
        from_route_edge_index: usize,
        to_route_edge_index: usize,
        gate: ManeuverGateHandle,
        remaining_travel: f64,
        final_speed: f64,
    },
    ParkingLeaveUnsafeFollower {
        vehicle: VehicleHandle,
        space: ParkingSpaceHandle,
        follower: VehicleHandle,
    },
    ParkingBindingInvariantViolation {
        stage: &'static str,
        vehicle: Option<VehicleHandle>,
        space: Option<ParkingSpaceHandle>,
    },
    ParkingTraversalBoundaryInvariant {
        vehicle: VehicleHandle,
        space: ParkingSpaceHandle,
        route: RouteHandle,
        route_edge_index: usize,
        remaining_travel: f64,
        final_speed: f64,
    },
    VehiclePhysicalOverlap {
        follower: VehicleHandle,
        leader: VehicleHandle,
        bumper_gap: f64,
    },
}

const _: () = assert!(std::mem::size_of::<TickInvariantError>() <= 64);
const _: () = assert!(std::mem::size_of::<Result<(), TickInvariantError>>() <= 72);
const _: () = assert!(std::mem::size_of::<Result<f64, TickInvariantError>>() <= 72);
const _: () = assert!(!std::mem::needs_drop::<TickInvariantError>());

const fn assert_copy<T: Copy>() {}
const _: () = assert_copy::<TickInvariantError>();
