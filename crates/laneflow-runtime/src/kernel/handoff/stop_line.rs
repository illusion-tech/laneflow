use laneflow_static_contract::LaneEdgeOrdinal;
use laneflow_static_network::{
    SharedNetworkRevision, StationCandidates,
    vehicle::{BodyEnvelope, Failure, HandoffQuery, HandoffWindow},
};

use super::RouteHandoffTrialError;

/// 对源侧沿行驶方向枚举停止线。前置普通机动已选的门由当拍裁决；从源出现
/// 起没有继续直行的门，车身须在其上游清空。最多读取八条映射边及首边的前驱。
pub(super) fn prove_source_clearance(
    revision: &SharedNetworkRevision,
    query: &HandoffQuery,
    window: HandoffWindow,
    body: BodyEnvelope,
    hits: &StationCandidates,
    work_limit: u32,
    selected_gate: impl Fn(LaneEdgeOrdinal, LaneEdgeOrdinal) -> bool,
) -> Result<u32, RouteHandoffTrialError> {
    let mapping = revision.station_mapping();
    // 首截面恰在内切点时只列下游边零点。其紧邻的前一边仍可能持有该停止线。
    let previous = hits
        .addresses()
        .next()
        .and_then(|hit| mapping.preceding_lane_edge(hit.edge));
    let mut edges = previous
        .into_iter()
        .chain(hits.addresses().map(|a| a.edge))
        .peekable();
    let increasing = query.rear_stations_nm[0] < query.rear_stations_nm[1];
    let mut upstream = true;
    let mut sweep = None;
    while let Some(edge) = edges.next() {
        if edge == window.edges[0] {
            upstream = false;
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
        let station = mapping
            .edge_end_station_nm(edge)
            .ok_or_else(|| unproven(Some(Failure::Unavailable), 0))?;
        let separated = |[lo, hi]: [u64; 2]| {
            if upstream {
                station < lo || hi < station
            } else if increasing {
                hi < station
            } else {
                station < lo
            }
        };
        // 原准入已约束整车几何站位。切点判断直接比较规范站位，避免毫米取整抹去分离。
        if separated(hits.stations_nm())
            || (upstream && edges.peek().is_some_and(|&next| selected_gate(edge, next)))
        {
            continue;
        }
        let sweep = sweep.get_or_insert_with(|| {
            let mut rear = query.rear_stations_nm;
            rear.sort_unstable();
            revision.handoff_swept_occupancy(query, window, body, rear, work_limit)
        });
        let [source, _] = sweep
            .sides
            .as_ref()
            .map_err(|&reason| unproven(Some(reason), sweep.work))?;
        if source.as_ref().is_some_and(|s| !separated(s.stations_nm())) {
            return Err(unproven(None, sweep.work));
        }
    }
    Ok(sweep.map_or(0, |s| s.work))
}
