use laneflow_static_contract::LaneEdgeOrdinal;
use laneflow_static_network::{
    SharedNetworkRevision, StationCandidates,
    vehicle::{BodyEnvelope, Failure, HandoffBinding, HandoffQuery, HandoffSweepReport},
};

use super::{RouteHandoffTrialError, sample::CursorRun};

/// 各侧至多检查八条投影边及首边的物理前驱。全程扫过一次即给出两侧车身范围。
pub(super) fn prove_clearance(
    revision: &SharedNetworkRevision,
    query: &HandoffQuery,
    boundary: &HandoffBinding,
    body: BodyEnvelope,
    sides: [(&StationCandidates, CursorRun<'_>); 2],
    work_limit: u32,
    selected_gate: impl Fn(CursorRun<'_>, LaneEdgeOrdinal, LaneEdgeOrdinal) -> bool,
) -> Result<u32, RouteHandoffTrialError> {
    let mapping = revision.station_mapping();
    let window = boundary.window();
    let mut sweep: Option<HandoffSweepReport> = None;
    for (side, (hits, run)) in sides.into_iter().enumerate() {
        // 内切点归下游边零点，停止线仍可能由上游边持有。
        let previous = hits
            .addresses()
            .next()
            .and_then(|hit| mapping.preceding_lane_edge(hit.edge));
        let mut edges = previous
            .into_iter()
            .chain(hits.addresses().map(|a| a.edge))
            .peekable();
        let mut must_clear_upstream = false;
        while let Some(edge) = edges.next() {
            if side == 0 && edge == window.edges[0] {
                // 自换出出现起不再有本路线选定的直行门，车身须在续行停止线前清空。
                must_clear_upstream = true;
            }
            if revision
                .traffic()
                .relations()
                .stop_line_for_edge(edge)
                .is_none()
            {
                continue;
            }
            let unproven =
                |reason, work| RouteHandoffTrialError::StopLineUnproven { edge, reason, work };
            let station = mapping.edge_end_station_mm(edge).ok_or_else(|| {
                unproven(
                    Some(Failure::Unavailable),
                    sweep.as_ref().map_or(0, |s| s.work),
                )
            })?;
            let separated = |hits: &StationCandidates| {
                let relation = hits.station_relation(station);
                if !must_clear_upstream {
                    !relation.is_eq()
                } else if boundary.increasing() {
                    relation.is_lt()
                } else {
                    relation.is_gt()
                }
            };
            if separated(hits)
                || (!must_clear_upstream
                    && edges
                        .peek()
                        .is_some_and(|&next| selected_gate(run, edge, next)))
            {
                continue;
            }
            let rear = query
                .rear_range()
                .ok_or_else(|| unproven(Some(Failure::InvalidInput), 0))?;
            let sweep = sweep.get_or_insert_with(|| {
                revision.handoff_swept_occupancy(query, window, body, rear, work_limit)
            });
            let occupied = sweep
                .sides
                .as_ref()
                .map_err(|&reason| unproven(Some(reason), sweep.work))?;
            if occupied[side].as_ref().is_some_and(|s| !separated(s)) {
                return Err(unproven(None, sweep.work));
            }
        }
    }
    Ok(sweep.map_or(0, |s| s.work))
}
