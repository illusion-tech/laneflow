//! 静态路线领域 Canonical LIR 记录。

use laneflow_static_contract::{
    LaneEdgeOrdinal, ManeuverGateOrdinal, ManeuverPathOrdinal, StaticRouteId, StaticRouteOrdinal,
    WaitingZoneOrdinal,
};

use crate::arena::TableRange;

use super::LirIdentityField;

pub(crate) struct LirStaticRouteTransition {
    pub(crate) maneuver_gate: Option<ManeuverGateOrdinal>,
}

pub(crate) struct LirManeuverOccurrence {
    pub(crate) maneuver_path: ManeuverPathOrdinal,
    pub(crate) entry_route_edge_index: u32,
    pub(crate) exit_route_edge_index: u32,
    pub(crate) gate_occurrences: TableRange<LirGateOccurrence>,
    pub(crate) waiting_zone_occurrences: TableRange<LirWaitingZoneOccurrence>,
}

pub(crate) struct LirGateOccurrence {
    pub(crate) maneuver_gate: ManeuverGateOrdinal,
    pub(crate) maneuver_occurrence_index: u32,
    pub(crate) from_route_edge_index: u32,
    pub(crate) next_gate_occurrence_index: Option<u32>,
    pub(crate) next_boundary_route_edge_index: u32,
    pub(crate) waiting_zone_occurrence_index: Option<u32>,
}

pub(crate) struct LirWaitingZoneOccurrence {
    pub(crate) waiting_zone: WaitingZoneOrdinal,
    pub(crate) maneuver_occurrence_index: u32,
    pub(crate) entry_gate_occurrence_index: u32,
    pub(crate) release_gate_occurrence_index: u32,
    pub(crate) entry_route_edge_index: u32,
    pub(crate) release_route_edge_index: u32,
}

pub(crate) struct LirStaticRoute {
    pub(crate) ordinal: StaticRouteOrdinal,
    pub(crate) stable_id: StaticRouteId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) edges: TableRange<LaneEdgeOrdinal>,
    pub(crate) transitions: TableRange<LirStaticRouteTransition>,
    pub(crate) maneuver_occurrences: TableRange<LirManeuverOccurrence>,
    pub(crate) gate_occurrences: TableRange<LirGateOccurrence>,
    pub(crate) waiting_zone_occurrences: TableRange<LirWaitingZoneOccurrence>,
}

/// 从稳定实体反查静态路线出现项；`occurrence_index` 是对应路线内的局部下标。
#[derive(Clone, Copy)]
pub(crate) struct LirRouteOccurrenceRef {
    pub(crate) static_route: StaticRouteOrdinal,
    pub(crate) occurrence_index: u32,
}
