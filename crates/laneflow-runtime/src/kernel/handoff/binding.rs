use std::sync::Arc;

use laneflow_static_contract::LaneEdgeOrdinal;
use laneflow_static_network::{
    SharedNetworkRevision,
    vehicle::{Failure, HandoffBinding, HandoffSourceEvidence, HandoffWindow},
};

use super::{BoundRouteHandoff, RouteHandoffOccurrence, RouteHandoffTrialError};
use crate::kernel::tables::{ManeuverOccurrence, RouteLegEnd, compile_route_leg, try_route_vec};
use crate::{RouteError, RouteRegisterInput, TrafficWorld};

impl TrafficWorld {
    /// 绑定候选路线中的一次横向交接。世界路线槽仍可用。
    ///
    /// # Errors
    ///
    /// 修订、出现下标、边界窗口、普通路线编译或注册容量不满足时返回相应错误。
    ///
    /// # Panics
    ///
    /// 世界因执行 panic 失效后调用会 panic。
    pub fn bind_route_handoff(
        &self,
        route: &RouteRegisterInput,
        occurrence: RouteHandoffOccurrence,
    ) -> Result<BoundRouteHandoff, RouteHandoffTrialError> {
        self.bind_route_handoffs(route, std::slice::from_ref(&occurrence))
    }

    /// 按源出现下标绑定候选路线中的全部交接，逐段编译窗口之间的普通路线。
    /// 每对出现最多有一个窗口，重复经过同一条边仍按路线下标区分。
    ///
    /// # Errors
    ///
    /// 窗口清单为空、重复或逆序，修订或边界不匹配、窗口越界、普通路线无法编译，
    /// 或登记容量、命令游标、内存预留不足时返回错误。失败时不发布部分绑定。
    /// 冲突容量错误以首个超限普通段为单位，已编译各段均计入该段的起始用量。
    ///
    /// # Panics
    ///
    /// 世界因执行 panic 失效后调用会 panic。
    pub fn bind_route_handoffs(
        &self,
        route: &RouteRegisterInput,
        occurrences: &[RouteHandoffOccurrence],
    ) -> Result<BoundRouteHandoff, RouteHandoffTrialError> {
        self.execution.assert_usable();
        let revision = &self.state.binding.revision;
        let edges = route.edges();
        self.state.preflight_route_registration(edges.len())?;
        if occurrences.len() > edges.len() - 1 {
            return Err(RouteHandoffTrialError::OccurrenceOutOfRange);
        }
        if occurrences.is_empty()
            || occurrences
                .windows(2)
                .any(|w| w[0].source_index >= w[1].source_index)
        {
            return Err(RouteHandoffTrialError::InvalidWindowOrder);
        }
        // 源下标严格递增。最后一项有后继即证明所有下标可表示且窗口数至多为 E−1。
        let edge_count =
            u32::try_from(edges.len()).map_err(|_| RouteHandoffTrialError::OccurrenceOutOfRange)?;
        let last = occurrences[occurrences.len() - 1].source_index;
        if last >= edge_count - 1 {
            return Err(RouteHandoffTrialError::OccurrenceOutOfRange);
        }
        let mut windows = try_route_vec(occurrences.len())?;
        for occurrence in occurrences {
            if occurrence.revision != revision.network_revision() {
                return Err(RouteHandoffTrialError::RevisionMismatch);
            }
            let i = occurrence.source_index as usize;
            let boundary = revision
                .bind_handoff_window(
                    occurrence.revision,
                    HandoffWindow {
                        edges: [edges[i], edges[i + 1]],
                        owner: occurrence.owner,
                        left_index: occurrence.left_index,
                        stations_nm: occurrence.stations_nm,
                    },
                )
                .map_err(RouteHandoffTrialError::Boundary)?;
            windows.push((occurrence.source_index, boundary));
        }
        let mut owned_edges = try_route_vec(edges.len())?;
        owned_edges.extend_from_slice(edges);
        self.compile_handoffs(owned_edges, windows)
    }

    /// 在目标世界逐个重验旧路线的全部交接窗口。保持完整出现顺序并重新编译普通机动。
    ///
    /// # Errors
    ///
    /// 两端来源无法证明任一窗口的物理对应，或目标路线不满足编译、容量、
    /// 命令游标或内存要求时，整条候选失败，两个世界均保留原状态。
    ///
    /// # Panics
    ///
    /// 世界因执行 panic 失效后调用会 panic。
    pub fn rebind_route_handoff(
        &self,
        old: &BoundRouteHandoff,
        old_sources: &HandoffSourceEvidence<'_>,
        new_sources: &HandoffSourceEvidence<'_>,
    ) -> Result<BoundRouteHandoff, RouteHandoffTrialError> {
        self.execution.assert_usable();
        let revision = &self.state.binding.revision;
        self.state.preflight_route_registration(old.edges.len())?;
        let mut windows = try_route_vec(old.windows.len())?;
        for &(index, boundary) in &old.windows {
            let rebound = revision
                .rebind_handoff_window(&old.revision, boundary, old_sources, new_sources)
                .map_err(RouteHandoffTrialError::Boundary)?;
            windows.push((index, rebound));
        }
        let mut edges = try_route_vec(old.edges.len())?;
        for &edge in &old.edges {
            let stable = old
                .revision
                .identity()
                .stable_id(edge)
                .ok_or(RouteError::UnknownEdge)?;
            edges.push(
                revision
                    .identity()
                    .ordinal(stable)
                    .ok_or(RouteError::UnknownEdge)?,
            );
        }
        self.compile_handoffs(edges, windows)
    }

    fn compile_handoffs(
        &self,
        edges: Vec<LaneEdgeOrdinal>,
        windows: Vec<(u32, HandoffBinding)>,
    ) -> Result<BoundRouteHandoff, RouteHandoffTrialError> {
        let revision = &self.state.binding.revision;
        for &(index, boundary) in &windows {
            let i = index as usize;
            let [source, target] = boundary.window().edges;
            if [edges[i], edges[i + 1]] != [source, target] {
                return Err(RouteHandoffTrialError::Boundary(Failure::Binding));
            }
            if revision
                .traffic()
                .maneuvers()
                .transition_candidates(source)
                .is_some_and(|candidates| candidates.iter().any(|hop| hop.successor() == target))
            {
                return Err(RouteError::ManeuverMismatch.into());
            }
        }
        let ends = windows
            .iter()
            .map(|w| w.0 as usize + 1)
            .chain([edges.len()]);
        let maneuvers = compile_ordinary_legs(
            revision,
            &edges,
            ends,
            self.state.committed.live_route_conflict_occurrence_count,
            self.state
                .binding
                .config
                .route_conflict_occurrence_capacity(),
        )?;
        Ok(BoundRouteHandoff {
            revision: Arc::clone(revision),
            edges,
            maneuvers,
            windows,
        })
    }
}

/// 段尾为已校验的独占下标，最后一段结束于路线末端。每个普通段按注册规则
/// 检查完整机动、门和资源；其整边距离只在段内成立，不能拼作交接距离。
fn compile_ordinary_legs(
    revision: &SharedNetworkRevision,
    edges: &[LaneEdgeOrdinal],
    ends: impl Iterator<Item = usize>,
    mut live_conflicts: u64,
    conflict_capacity: u64,
) -> Result<Vec<ManeuverOccurrence>, RouteError> {
    let mut maneuvers = Vec::new();
    let mut start = 0;
    for end in ends {
        let leg_end = if end == edges.len() {
            RouteLegEnd::Destination
        } else {
            RouteLegEnd::BoundHandoffSource
        };
        let leg = compile_route_leg(
            revision,
            &edges[start..end],
            leg_end,
            live_conflicts,
            conflict_capacity,
        )?;
        live_conflicts += leg.conflicts.len() as u64;
        maneuvers
            .try_reserve(leg.maneuvers.len())
            .map_err(|_| RouteError::AllocationFailed)?;
        let offset = start as u32;
        maneuvers.extend(leg.maneuvers.into_iter().map(|m| ManeuverOccurrence {
            path: m.path,
            entry_route_edge_index: m.entry_route_edge_index + offset,
            exit_route_edge_index: m.exit_route_edge_index + offset,
        }));
        start = end;
    }
    Ok(maneuvers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::cutover_migration::tests::conflict_scale_revision;
    use laneflow_static_contract::{ManeuverPathOrdinal, ParticipantStreamOrdinal};

    #[test]
    fn only_a_leg_ending_at_a_handoff_accepts_a_stop_line() {
        let revision = conflict_scale_revision();
        let traffic = revision.traffic();
        let path = traffic
            .maneuvers()
            .maneuver_path(ManeuverPathOrdinal::from_raw(0))
            .unwrap();
        let &[entry, internal, exit] = path.edges() else {
            panic!("fixture must have a single junction-owned internal edge");
        };
        assert!(traffic.relations().stop_line_for_edge(entry).is_some());
        let compile = |edges: &[LaneEdgeOrdinal], ends: &[usize]| {
            compile_ordinary_legs(&revision, edges, ends.iter().copied(), 0, 0)
        };
        assert_eq!(compile(&[entry, exit], &[1, 2]), Ok(Vec::new()));
        assert_eq!(
            compile(&[exit, entry], &[1, 2]),
            Err(RouteError::ManeuverMismatch)
        );
        assert_eq!(
            compile(&[entry, internal, exit], &[2, 3]),
            Err(RouteError::ManeuverMismatch)
        );
    }

    #[test]
    fn ordinary_legs_preserve_maneuver_coordinates_and_share_the_conflict_budget() {
        let revision = conflict_scale_revision();
        let stream = revision
            .conflict()
            .participant_stream(ParticipantStreamOrdinal::from_raw(0))
            .unwrap();
        let path = stream.maneuver_path();
        let ordinary = revision
            .traffic()
            .maneuvers()
            .maneuver_path(path)
            .unwrap()
            .edges();
        assert_eq!(ordinary.len(), 3);
        let edges = ordinary.repeat(3);
        let compile = |ends: &[usize], capacity| {
            compile_ordinary_legs(&revision, &edges, ends.iter().copied(), 1, capacity)
        };
        let maneuvers = compile(&[3, 6, 9], 4).unwrap();
        assert_eq!(maneuvers.len(), 3);
        for (i, occurrence) in maneuvers.into_iter().enumerate() {
            assert_eq!(occurrence.path, path);
            assert_eq!(occurrence.entry_route_edge_index, i as u32 * 3);
            assert_eq!(occurrence.exit_route_edge_index, i as u32 * 3 + 2);
        }
        assert!(matches!(
            compile(&[3, 6, 9], 3),
            Err(RouteError::ConflictOccurrenceCapacityExceeded { .. })
        ));
        assert_eq!(compile(&[3, 5, 9], 4), Err(RouteError::ManeuverMismatch));
    }
}
