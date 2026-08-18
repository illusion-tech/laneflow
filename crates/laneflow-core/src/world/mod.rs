//! Core world 与 fixed-step orchestration。

use indexmap::IndexMap;

use crate::{
    access::{AccessCell, AccessEffect, AccessRegistry},
    command_spatial::{CommandOccupant, CommandSpatialIndex},
    cross_section::CrossSectionRegistry,
    error::{CoreError, TickInvariantError, WaitingZoneError},
    event::{
        CoreEvent, ParkingReservationReleasedEvent, SignalGroupAspectChangedEvent,
        SignalPhaseChangedEvent, VehicleChangedEdgeEvent, VehicleCompletedRouteEvent,
        VehicleFollowingSafetyProjectionAppliedEvent, VehicleParkingArrivalReachedEvent,
        VehicleParkingStopProjectionAppliedEvent, VehicleSignalStopProjectionAppliedEvent,
        VehicleSpeedLimitProjectionAppliedEvent,
    },
    graph::LaneGraph,
    handle::{
        AccessRuleHandle, EdgeHandle, ManeuverGateHandle, RouteHandle, SignalControllerHandle,
        SignalGroupHandle, VehicleHandle, VehicleProfileHandle,
    },
    id::validate_external_id,
    junction::JunctionRegistry,
    longitudinal::{
        LeaderKinematics, LongitudinalMotion, LongitudinalScratch, SpeedLimitConstraint,
        compute_motion, emergency_min_travel,
    },
    numeric_policy::{
        EDGE_BOUNDARY_TOLERANCE_METERS, LONGITUDINAL_CONSTRAINT_TOLERANCE_METERS,
        MINIMUM_GAP_TOLERANCE_METERS, PHYSICAL_GAP_TOLERANCE_METERS,
        WAITING_ZONE_STORAGE_TOLERANCE_METERS, computed_speed_is_above_near_zero,
        is_edge_boundary_remainder_zero, longitudinal_constraint_reached,
        longitudinal_positions_match, normalize_physical_gap, physical_gap_is_overlap,
    },
    occupancy::{LeaderObservation, OccupancyScratch, Occupant},
    parking::{
        LeaveParkingInput, ParkedVehicleSpawnInput, ParkedVehicleSpawnRecord,
        ParkingApproachTarget, ParkingBindingKind, ParkingCommandEffect, ParkingCommandKind,
        ParkingCommitRecord, ParkingLeaveRecord, ParkingRegistry, ParkingReleaseReason,
        ParkingReleaseRecord, ParkingReservationCancellationRecord, ParkingReservationRecord,
        ParkingRuntimeState, ParkingSnapshot, ParkingSpaceState, ParkingStopConstraint,
        RebindReservedVehicleRouteInput, ReservedVehicleRouteRebindRecord,
        RuntimeVehicleParkingBinding,
    },
    participant_class::ParticipantClassRegistry,
    profile::{VehicleProfile, VehicleProfileRegistry},
    route::{Route, RouteRemoveRecord},
    route_distance::{BoundedDistance, RouteDistanceIndex, RouteDistanceQuery},
    signal::{
        ManeuverGateSignalState, ManeuverGateState, SignalControllerState, SignalGroupSnapshot,
        SignalLayerPermission, SignalRegistry, SignalRuntimeScratch, SignalRuntimeState,
        SignalStopConstraint,
    },
    step_probe::{NoOpProbe, StepProbe},
    time::{StepResult, TickInput},
    traffic::{
        CompiledRoute, GateOccurrence, InitialTrafficData, ManeuverOccurrence,
        WaitingZoneOccurrence, compile_route,
    },
    vehicle::{
        Acceleration, EdgeProgress, Speed, VehicleDespawnRecord, VehicleReplaceBlock,
        VehicleReplaceBlockerPosition, VehicleReplaceExternalId, VehicleReplaceInput,
        VehicleReplaceOutcome, VehicleReplaceRecord, VehicleSpawnInput, VehicleState,
        VehicleStatus,
    },
    waiting::WaitingRegistry,
};

mod parking_commands;
mod route_lifecycle;
mod route_queries;
mod signal_queries;
mod state;
mod support;
mod tick;
mod tick_advance;
mod tick_longitudinal;
mod tick_overlap;
mod tick_spatial;
mod vehicle_lifecycle;

pub use state::CoreWorld;
use support::*;

#[cfg(test)]
mod occupancy_tests;

#[cfg(test)]
mod retained_memory_tests;

#[cfg(test)]
mod event_merge_research_tests;

#[cfg(test)]
mod partitioned_occupancy_research_tests;

#[cfg(test)]
mod selective_read_research_tests;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod retained_memory;
