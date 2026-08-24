//! 控制领域 Canonical LIR 记录：停止线、机动门与等待区。

use laneflow_static_contract::{
    LaneEdgeOrdinal, ManeuverGateId, ManeuverGateOrdinal, ManeuverPathOrdinal, StopLineId,
    StopLineOrdinal, WaitingZoneId, WaitingZoneOrdinal,
};

use crate::arena::TableRange;

use super::{LirIdentityField, LirRouteOccurrenceRef, LirSignalControl};

pub(crate) struct LirStopLine {
    pub(crate) ordinal: StopLineOrdinal,
    pub(crate) stable_id: StopLineId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) lane_edge: LaneEdgeOrdinal,
    pub(crate) maneuver_gates: TableRange<ManeuverGateOrdinal>,
}

pub(crate) struct LirManeuverGate {
    pub(crate) ordinal: ManeuverGateOrdinal,
    pub(crate) stable_id: ManeuverGateId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) maneuver_path: ManeuverPathOrdinal,
    pub(crate) transition_index: u32,
    pub(crate) stop_line: StopLineOrdinal,
    pub(crate) signal_control: LirSignalControl,
    pub(crate) static_route_occurrences: TableRange<LirRouteOccurrenceRef>,
}

pub(crate) struct LirWaitingZone {
    pub(crate) ordinal: WaitingZoneOrdinal,
    pub(crate) stable_id: WaitingZoneId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) maneuver_path: ManeuverPathOrdinal,
    pub(crate) entry_gate: ManeuverGateOrdinal,
    pub(crate) release_gate: ManeuverGateOrdinal,
    pub(crate) max_occupancy: u32,
    pub(crate) static_route_occurrences: TableRange<LirRouteOccurrenceRef>,
}
