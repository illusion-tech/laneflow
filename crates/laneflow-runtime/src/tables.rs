use laneflow_static_contract::{
    AccessEffect, LaneEdgeOrdinal, ManeuverPathOrdinal, ParkingSpaceOrdinal,
    ParticipantClassOrdinal, StaticRouteOrdinal, VehicleProfileOrdinal,
};
use laneflow_static_network::{AccessCell, SharedTrafficNetwork};

use crate::{RouteError, RouteHandle, SpawnError, VehicleHandle};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ManeuverOccurrence {
    pub path: ManeuverPathOrdinal,
    pub exit_route_edge_index: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CompiledRoute {
    pub edges: Box<[LaneEdgeOrdinal]>,
    pub maneuvers: Box<[ManeuverOccurrence]>,
}

#[derive(Clone, Debug)]
pub(crate) struct DynamicRouteSlot {
    pub generation: u32,
    pub compiled: Option<CompiledRoute>,
    pub live_vehicles: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VehicleStatus {
    Active,
    Parked,
    Completed,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct VehicleState {
    pub handle: VehicleHandle,
    pub profile: VehicleProfileOrdinal,
    pub class: ParticipantClassOrdinal,
    pub route: RouteHandle,
    pub route_edge_index: u32,
    pub progress: f64,
    pub speed: f64,
    pub length: f64,
    pub status: VehicleStatus,
    pub parking: Option<ParkingSpaceOrdinal>,
}

#[derive(Clone, Debug)]
pub(crate) struct VehicleSlot {
    pub generation: u32,
    pub state: Option<VehicleState>,
}

pub(crate) fn compile_dynamic_route(
    traffic: &SharedTrafficNetwork,
    edges: &[LaneEdgeOrdinal],
) -> Result<CompiledRoute, RouteError> {
    if edges.is_empty() {
        return Err(RouteError::EmptySequence);
    }
    let count = traffic.lane_edge_count();
    for edge in edges {
        if edge.raw() >= count {
            return Err(RouteError::UnknownEdge);
        }
    }
    for pair in edges.windows(2) {
        let Some(successors) = traffic.successors(pair[0]) else {
            return Err(RouteError::UnknownEdge);
        };
        if !successors.contains(&pair[1]) {
            return Err(RouteError::Disconnected);
        }
    }

    let mut maneuvers: Vec<ManeuverOccurrence> = Vec::new();
    let network = traffic.maneuvers();
    for (index, pair) in edges.windows(2).enumerate() {
        let Some(candidates) = network.transition_candidates(pair[0]) else {
            continue;
        };
        let Some(candidate) = candidates
            .iter()
            .find(|candidate| candidate.successor() == pair[1])
        else {
            continue;
        };
        let path = candidate.maneuver_path();
        let entry = u32::try_from(index).expect("route edge index fits u32");
        let exit = entry
            .checked_add(1)
            .expect("route edge index increment fits u32");
        if let Some(last) = maneuvers.last_mut()
            && last.path == path
            && last.exit_route_edge_index == entry
        {
            last.exit_route_edge_index = exit;
            continue;
        }
        maneuvers.push(ManeuverOccurrence {
            path,
            exit_route_edge_index: exit,
        });
    }

    Ok(CompiledRoute {
        edges: edges.to_vec().into_boxed_slice(),
        maneuvers: maneuvers.into_boxed_slice(),
    })
}

pub(crate) fn route_access_denied(
    traffic: &SharedTrafficNetwork,
    class: ParticipantClassOrdinal,
    edges: &[LaneEdgeOrdinal],
    cursor: usize,
    maneuvers: impl Iterator<Item = (ManeuverPathOrdinal, u32)>,
) -> bool {
    for edge in edges.iter().skip(cursor) {
        if matches!(
            traffic.relations().edge_access(*edge, class),
            Some(AccessCell::Decided {
                effect: AccessEffect::Deny,
                ..
            })
        ) {
            return true;
        }
    }
    let cursor = u32::try_from(cursor).expect("route cursor fits u32");
    for (path, exit_index) in maneuvers {
        if exit_index <= cursor {
            continue;
        }
        if matches!(
            traffic.relations().path_access(path, class),
            Some(AccessCell::Decided {
                effect: AccessEffect::Deny,
                ..
            })
        ) {
            return true;
        }
    }
    false
}

pub(crate) fn bumpers_overlap(a_front: f64, a_length: f64, b_front: f64, b_length: f64) -> bool {
    let a_rear = a_front - a_length;
    let b_rear = b_front - b_length;
    a_rear < b_front && b_rear < a_front
}

pub(crate) fn occupancy_intervals(
    lengths: &[f64],
    edges: &[LaneEdgeOrdinal],
    mut index: usize,
    mut end: f64,
    mut remaining: f64,
) -> Option<Vec<(LaneEdgeOrdinal, f64, f64)>> {
    let mut intervals = Vec::new();
    while remaining > 1e-12 {
        let edge = *edges.get(index)?;
        let edge_length = *lengths.get(edge.index())?;
        let start = (end - remaining).max(0.0);
        let hi = end.min(edge_length);
        if hi > start {
            intervals.push((edge, start, hi));
        }
        remaining -= hi - start;
        if remaining <= 1e-12 || index == 0 {
            break;
        }
        index -= 1;
        end = *lengths.get(edges.get(index)?.index())?;
    }
    Some(intervals)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn bodies_overlap(
    lengths: &[f64],
    a_edges: &[LaneEdgeOrdinal],
    a_index: usize,
    a_progress: f64,
    a_length: f64,
    b_edges: &[LaneEdgeOrdinal],
    b_index: usize,
    b_progress: f64,
    b_length: f64,
) -> bool {
    if a_edges.get(a_index) == b_edges.get(b_index)
        && bumpers_overlap(a_progress, a_length, b_progress, b_length)
    {
        return true;
    }
    let Some(left) = occupancy_intervals(lengths, a_edges, a_index, a_progress, a_length) else {
        return false;
    };
    let Some(right) = occupancy_intervals(lengths, b_edges, b_index, b_progress, b_length) else {
        return false;
    };
    left.iter().any(|(edge, a_lo, a_hi)| {
        right
            .iter()
            .any(|(other, b_lo, b_hi)| *edge == *other && *a_lo < *b_hi && *b_lo < *a_hi)
    })
}

pub(crate) fn static_route_ordinal(handle: RouteHandle) -> Option<StaticRouteOrdinal> {
    handle
        .is_static()
        .then(|| StaticRouteOrdinal::from_raw(handle.index()))
}

pub(crate) fn remaining_along_route(
    lengths: &[f64],
    edges: &[LaneEdgeOrdinal],
    from_index: usize,
    from_progress: f64,
    to_index: usize,
    to_progress: f64,
) -> Option<f64> {
    if to_index < from_index || (to_index == from_index && to_progress < from_progress) {
        return None;
    }
    if to_index == from_index {
        return Some(to_progress - from_progress);
    }
    let from_edge = *edges.get(from_index)?;
    let mut distance = lengths.get(from_edge.index())? - from_progress;
    for edge in edges.get(from_index + 1..to_index)? {
        distance += *lengths.get(edge.index())?;
    }
    Some(distance + to_progress)
}

pub(crate) fn remaining_to_route_end(
    lengths: &[f64],
    edges: &[LaneEdgeOrdinal],
    from_index: usize,
    from_progress: f64,
) -> Option<f64> {
    let last = edges.len().checked_sub(1)?;
    let last_edge = *edges.get(last)?;
    remaining_along_route(
        lengths,
        edges,
        from_index,
        from_progress,
        last,
        *lengths.get(last_edge.index())?,
    )
}

pub(crate) fn spawn_motion_error(progress: f64, speed: f64) -> Option<SpawnError> {
    if !progress.is_finite() || progress < 0.0 {
        return Some(SpawnError::InvalidProgress);
    }
    if !speed.is_finite() || speed < 0.0 {
        return Some(SpawnError::InvalidSpeed);
    }
    None
}
