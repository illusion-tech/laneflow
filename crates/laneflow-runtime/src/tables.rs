use laneflow_static_contract::{
    AccessEffect, LaneEdgeOrdinal, ManeuverGateOrdinal, ManeuverPathOrdinal, ParkingSpaceOrdinal,
    ParticipantClassOrdinal, StaticRouteOrdinal, VehicleProfileOrdinal,
};
use laneflow_static_network::{AccessCell, SharedManeuverNetwork, SharedTrafficNetwork};

use crate::{RouteError, RouteHandle, SpawnError, VehicleHandle};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ManeuverOccurrence {
    pub path: ManeuverPathOrdinal,
    pub entry_route_edge_index: u32,
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
    let transition_len = edges.len().saturating_sub(1);
    let mut next_entry = 0;
    for entry_index in 0..transition_len {
        if entry_index < next_entry {
            continue;
        }
        let Some(path_ordinal) = unique_entry_path_match(
            network,
            edges[entry_index],
            edges[entry_index + 1],
            &edges[entry_index..],
        )?
        else {
            continue;
        };
        let path = network
            .maneuver_path(path_ordinal)
            .ok_or(RouteError::ManeuverMismatch)?;
        let exit_index = entry_index
            .checked_add(path.edges().len())
            .and_then(|value| value.checked_sub(1))
            .ok_or(RouteError::ManeuverMismatch)?;
        if exit_index >= edges.len() {
            return Err(RouteError::ManeuverMismatch);
        }
        maneuvers.push(ManeuverOccurrence {
            path: path_ordinal,
            entry_route_edge_index: u32::try_from(entry_index).expect("route edge index fits u32"),
            exit_route_edge_index: u32::try_from(exit_index).expect("route edge index fits u32"),
        });
        next_entry = exit_index;
    }

    Ok(CompiledRoute {
        edges: edges.to_vec().into_boxed_slice(),
        maneuvers: maneuvers.into_boxed_slice(),
    })
}

/// 在机动路径入口跳上，用剩余边序列唯一匹配完整 `path.edges()` 前缀。
///
/// 与静态路线 `unique_entry_path_match` 同一规则：只认 `transition_index == 0`；
/// 多条不同 path 都匹配则歧义；有入口候选但对不上完整路径则失败。
pub(crate) fn unique_entry_path_match(
    network: &SharedManeuverNetwork,
    from: LaneEdgeOrdinal,
    to: LaneEdgeOrdinal,
    remaining: &[LaneEdgeOrdinal],
) -> Result<Option<ManeuverPathOrdinal>, RouteError> {
    let Some(candidates) = network.transition_candidates(from) else {
        return Ok(None);
    };
    let mut entry_paths = Vec::new();
    for candidate in candidates {
        if candidate.successor() != to || candidate.transition_index() != 0 {
            continue;
        }
        let path = network
            .maneuver_path(candidate.maneuver_path())
            .ok_or(RouteError::ManeuverMismatch)?;
        entry_paths.push((candidate.maneuver_path(), path.edges()));
    }
    unique_entry_path_match_filtered(remaining, entry_paths)
}

fn unique_entry_path_match_filtered<'a>(
    remaining: &[LaneEdgeOrdinal],
    entry_paths: impl IntoIterator<Item = (ManeuverPathOrdinal, &'a [LaneEdgeOrdinal])>,
) -> Result<Option<ManeuverPathOrdinal>, RouteError> {
    let mut matched = None;
    let mut saw_entry = false;
    for (path, edges) in entry_paths {
        saw_entry = true;
        if remaining.starts_with(edges) {
            match matched {
                None => matched = Some(path),
                Some(first) if first != path => return Err(RouteError::AmbiguousManeuver),
                Some(_) => {}
            }
        }
    }
    if !saw_entry {
        return Ok(None);
    }
    matched.ok_or(RouteError::ManeuverMismatch).map(Some)
}

/// 动态路线已编译 occurrence 上，即将跨越的 hop 对应的闸。
pub(crate) fn compiled_hop_gate(
    network: &SharedManeuverNetwork,
    compiled: &CompiledRoute,
    hop_index: usize,
    from: LaneEdgeOrdinal,
    to: LaneEdgeOrdinal,
) -> Option<ManeuverGateOrdinal> {
    let hop = u32::try_from(hop_index).ok()?;
    let occurrence = compiled.maneuvers.iter().find(|occurrence| {
        hop >= occurrence.entry_route_edge_index && hop < occurrence.exit_route_edge_index
    })?;
    let transition_index = hop.checked_sub(occurrence.entry_route_edge_index)?;
    network
        .transition_candidates(from)?
        .iter()
        .find(|candidate| {
            candidate.successor() == to
                && candidate.maneuver_path() == occurrence.path
                && candidate.transition_index() == transition_index
        })?
        .maneuver_gate()
}

fn access_cell_denied(cell: Option<AccessCell>) -> bool {
    match cell {
        Some(AccessCell::Unconstrained) => false,
        Some(AccessCell::Decided {
            effect: AccessEffect::Allow,
            ..
        }) => false,
        Some(AccessCell::Decided {
            effect: AccessEffect::Deny,
            ..
        }) => true,
        Some(_) | None => true,
    }
}

pub(crate) fn route_access_denied(
    traffic: &SharedTrafficNetwork,
    class: ParticipantClassOrdinal,
    edges: &[LaneEdgeOrdinal],
    cursor: usize,
    maneuvers: impl Iterator<Item = (ManeuverPathOrdinal, u32)>,
) -> bool {
    for edge in edges.iter().skip(cursor) {
        if access_cell_denied(traffic.relations().edge_access(*edge, class)) {
            return true;
        }
    }
    let cursor = u32::try_from(cursor).expect("route cursor fits u32");
    for (path, exit_index) in maneuvers {
        if exit_index <= cursor {
            continue;
        }
        if access_cell_denied(traffic.relations().path_access(path, class)) {
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

#[cfg(test)]
mod unique_entry_path_match_tests {
    use super::*;
    use laneflow_static_contract::LaneEdgeOrdinal;

    fn edge(raw: u32) -> LaneEdgeOrdinal {
        LaneEdgeOrdinal::from_raw(raw)
    }

    fn path(raw: u32) -> ManeuverPathOrdinal {
        ManeuverPathOrdinal::from_raw(raw)
    }

    #[test]
    fn remaining_abd_selects_path_abd_not_abc() {
        let abc = [edge(0), edge(1), edge(2)];
        let abd = [edge(0), edge(1), edge(3)];
        let remaining = [edge(0), edge(1), edge(3)];
        let matched = unique_entry_path_match_filtered(
            &remaining,
            [(path(0), abc.as_slice()), (path(1), abd.as_slice())],
        )
        .expect("unique");
        assert_eq!(matched, Some(path(1)));
    }

    #[test]
    fn remaining_abc_selects_path_abc() {
        let abc = [edge(0), edge(1), edge(2)];
        let abd = [edge(0), edge(1), edge(3)];
        let remaining = [edge(0), edge(1), edge(2)];
        let matched = unique_entry_path_match_filtered(
            &remaining,
            [(path(0), abc.as_slice()), (path(1), abd.as_slice())],
        )
        .expect("unique");
        assert_eq!(matched, Some(path(0)));
    }

    #[test]
    fn two_prefix_matches_are_ambiguous() {
        let short = [edge(0), edge(1), edge(2)];
        let long = [edge(0), edge(1), edge(2), edge(3)];
        let remaining = [edge(0), edge(1), edge(2), edge(3)];
        assert_eq!(
            unique_entry_path_match_filtered(
                &remaining,
                [(path(0), short.as_slice()), (path(1), long.as_slice())],
            ),
            Err(RouteError::AmbiguousManeuver)
        );
    }

    #[test]
    fn entry_without_complete_path_is_mismatch() {
        let abc = [edge(0), edge(1), edge(2)];
        let abd = [edge(0), edge(1), edge(3)];
        let remaining = [edge(0), edge(1)];
        assert_eq!(
            unique_entry_path_match_filtered(
                &remaining,
                [(path(0), abc.as_slice()), (path(1), abd.as_slice())],
            ),
            Err(RouteError::ManeuverMismatch)
        );
    }

    #[test]
    fn no_entry_candidate_is_not_a_maneuver_start() {
        let remaining = [edge(0), edge(1)];
        assert_eq!(unique_entry_path_match_filtered(&remaining, []), Ok(None));
    }

    #[test]
    fn duplicate_same_path_is_unique() {
        let abc = [edge(0), edge(1), edge(2)];
        let remaining = [edge(0), edge(1), edge(2)];
        let matched = unique_entry_path_match_filtered(
            &remaining,
            [(path(0), abc.as_slice()), (path(0), abc.as_slice())],
        )
        .expect("unique");
        assert_eq!(matched, Some(path(0)));
    }
}

#[cfg(test)]
mod compile_dynamic_route_tests {
    use super::*;
    use std::sync::Arc;

    use laneflow_format::{FormatLimits, check_canonical_network_input_v1};
    use laneflow_static_network::{
        SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
        build_shared_network_revision,
    };

    const FULL_SPATIAL: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable-v1/lfca-v1-full-spatial/expected.lfca"
    );

    fn revision() -> Arc<laneflow_static_network::SharedNetworkRevision> {
        let input = check_canonical_network_input_v1(FULL_SPATIAL, FormatLimits::V1_HARD).unwrap();
        build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::RetainAvailable,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .unwrap()
    }

    #[test]
    fn s1_fixture_compiles_the_single_maneuver_path() {
        let revision = revision();
        let traffic = revision.traffic();
        let path = traffic
            .maneuvers()
            .maneuver_path(ManeuverPathOrdinal::from_raw(0))
            .expect("fixture path");
        let compiled = compile_dynamic_route(traffic, path.edges()).expect("compile");
        assert_eq!(compiled.maneuvers.len(), 1);
        assert_eq!(compiled.maneuvers[0].path, ManeuverPathOrdinal::from_raw(0));
        assert_eq!(compiled.maneuvers[0].entry_route_edge_index, 0);
        assert_eq!(
            compiled.maneuvers[0].exit_route_edge_index,
            u32::try_from(path.edges().len() - 1).expect("path length")
        );
        let first = path.edges()[0];
        let second = path.edges()[1];
        let gate = compiled_hop_gate(traffic.maneuvers(), &compiled, 0, first, second);
        assert_eq!(gate, path.maneuver_gates().first().copied());
    }
}
