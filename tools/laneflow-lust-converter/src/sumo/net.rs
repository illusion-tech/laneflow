//! Parsed SUMO network model (lanes, edges, connections, location).

use crate::sumo::decimal::ExactDecimal;

/// Pinned LuST `<location>` lexical anchors from `real-road-workloads.md` §2.2.
pub const LUST_NET_OFFSET: &str = "-285448.66,-5492398.13";
pub const LUST_CONV_BOUNDARY: &str = "0.00,0.00,13613.76,11455.04";

/// Spatial frame ID fixed by §3.2.
pub const LUST_FRAME_ID: &str = "lust-v2.0-c4bd5bd3-convboundary-center";

/// External ID namespace prefix for every SUMO identity.
pub const SUMO_ID_PREFIX: &str = "sumo:";

/// Parsed SUMO network used by topology conversion.
#[derive(Clone, Debug)]
pub struct SumoNetwork {
    pub location: SumoLocation,
    pub lanes: Vec<SumoLane>,
    pub connections: Vec<SumoConnection>,
}

/// `<location>` attributes as exact decimals.
#[derive(Clone, Debug)]
pub struct SumoLocation {
    pub net_offset: (ExactDecimal, ExactDecimal),
    pub conv_boundary: (ExactDecimal, ExactDecimal, ExactDecimal, ExactDecimal),
    pub net_offset_raw: String,
    pub conv_boundary_raw: String,
}

impl SumoLocation {
    /// Canonical origin = midpoint of projected `convBoundary`.
    pub fn canonical_origin(&self) -> crate::Result<(ExactDecimal, ExactDecimal)> {
        let (min_x, min_y, max_x, max_y) = self.conv_boundary;
        let projected_min_x = min_x.checked_sub(self.net_offset.0)?;
        let projected_min_y = min_y.checked_sub(self.net_offset.1)?;
        let projected_max_x = max_x.checked_sub(self.net_offset.0)?;
        let projected_max_y = max_y.checked_sub(self.net_offset.1)?;
        Ok((
            projected_min_x.midpoint(projected_max_x)?,
            projected_min_y.midpoint(projected_max_y)?,
        ))
    }

    /// Whether this location matches the pinned LuST anchors.
    pub fn matches_lust_anchors(&self) -> bool {
        self.net_offset_raw == LUST_NET_OFFSET && self.conv_boundary_raw == LUST_CONV_BOUNDARY
    }
}

/// One SUMO `<lane>` (external or internal).
#[derive(Clone, Debug)]
pub struct SumoLane {
    pub id: String,
    pub edge_id: String,
    pub index: u32,
    pub length: ExactDecimal,
    pub speed: ExactDecimal,
    pub shape: Vec<(ExactDecimal, ExactDecimal)>,
    pub function_internal: bool,
}

impl SumoLane {
    /// LaneFlow external ID (`sumo:{laneId}`).
    pub fn laneflow_id(&self) -> String {
        format!("{SUMO_ID_PREFIX}{}", self.id)
    }
}

/// One SUMO `<connection>`.
#[derive(Clone, Debug)]
pub struct SumoConnection {
    pub from_edge_id: String,
    pub to_edge_id: String,
    pub from_lane: u32,
    pub to_lane: u32,
    pub via_lane_ids: Vec<String>,
}

impl SumoNetwork {
    /// Count external (non-internal) lanes.
    pub fn external_lane_count(&self) -> usize {
        self.lanes
            .iter()
            .filter(|lane| !lane.function_internal)
            .count()
    }

    /// Count distinct external edge IDs.
    pub fn external_edge_count(&self) -> usize {
        let mut ids: Vec<&str> = self
            .lanes
            .iter()
            .filter(|lane| !lane.function_internal)
            .map(|lane| lane.edge_id.as_str())
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids.len()
    }
}
