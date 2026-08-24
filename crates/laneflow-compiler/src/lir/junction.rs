//! 路口领域 Canonical LIR 记录。

use laneflow_static_contract::{
    JunctionId, JunctionOrdinal, LaneEdgeOrdinal, ManeuverGateOrdinal, ManeuverPathId,
    ManeuverPathOrdinal, MovementId, MovementOrdinal, WaitingZoneOrdinal,
};

use crate::arena::TableRange;

use super::{LirIdentityField, LirRouteOccurrenceRef};

pub(crate) struct LirJunction {
    pub(crate) ordinal: JunctionOrdinal,
    pub(crate) stable_id: JunctionId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) movements: TableRange<MovementOrdinal>,
}

pub(crate) struct LirMovement {
    pub(crate) ordinal: MovementOrdinal,
    pub(crate) stable_id: MovementId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) junction: JunctionOrdinal,
    pub(crate) directed_entry_approach_key: Box<str>,
    pub(crate) directed_exit_approach_key: Box<str>,
    pub(crate) maneuver_paths: TableRange<ManeuverPathOrdinal>,
}

pub(crate) struct LirManeuverPath {
    pub(crate) ordinal: ManeuverPathOrdinal,
    pub(crate) stable_id: ManeuverPathId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) movement: MovementOrdinal,
    /// 完整 `entry + internal + exit` 序列。
    pub(crate) edges: TableRange<LaneEdgeOrdinal>,
    pub(crate) maneuver_gates: TableRange<ManeuverGateOrdinal>,
    pub(crate) waiting_zones: TableRange<WaitingZoneOrdinal>,
    pub(crate) static_route_occurrences: TableRange<LirRouteOccurrenceRef>,
}

pub(crate) struct LirJunctionInternalEdge {
    pub(crate) edge: LaneEdgeOrdinal,
    pub(crate) junction: JunctionOrdinal,
}
