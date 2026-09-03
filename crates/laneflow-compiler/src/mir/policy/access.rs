//! 策略校验复用 Access 的 class/target/priority 顺序，两个准入平面分别求值。
use super::super::*;
use crate::AccessEffect;

#[derive(Clone, Copy)]
pub(super) struct Entry {
    plane: u8,
    target: u32,
    rule: u32,
    specificity: u8,
}
pub(super) struct AccessIndex {
    entries: Vec<Entry>,
}

impl AccessIndex {
    pub(super) fn build(
        unit: &CompilationUnit,
        mir: &MirUnit,
        prior_scratch: u64,
    ) -> Result<Self, DiagnosticBundle> {
        let mut count = 0_u64;
        visit(mir, |_| {
            count = count.saturating_add(1);
        });
        let bytes = count.saturating_mul(size_of::<Entry>() as u64);
        super::validation::budget(unit, mir, prior_scratch.saturating_add(bytes), count)?;
        let mut entries = Vec::with_capacity(usize::try_from(count).map_err(|_| {
            DiagnosticBundle::single(Diagnostic::compile_limit_exceeded(
                CompileLimitDimension::RelationOccurrenceCount,
                u32::MAX.into(),
                count,
            ))
        })?);
        visit(mir, |entry| entries.push(entry));
        entries.sort_unstable_by_key(|e| (e.plane, e.target, e.rule));
        Ok(Self { entries })
    }
    pub(super) fn bytes(&self) -> u64 {
        (self.entries.len() as u64).saturating_mul(size_of::<Entry>() as u64)
    }
    fn allows(&self, mir: &MirUnit, plane: u8, target: u32, class: MirParticipantClassKey) -> bool {
        let start = self
            .entries
            .partition_point(|e| (e.plane, e.target) < (plane, target));
        let end = self
            .entries
            .partition_point(|e| (e.plane, e.target) <= (plane, target));
        let mut winner = None;
        for e in &self.entries[start..end] {
            let r = &mir.access_rules[e.rule as usize];
            let depth = mir.access_rule_participant_classes[r.participant_classes.as_usize_range()]
                .iter()
                .filter_map(|s| super::validation::class_depth(mir, s.participant_class, class))
                .max();
            if let Some(depth) = depth {
                let rank = (depth, e.specificity, r.priority);
                // 既有 HIR 已拒绝同组合 allow/deny 歧义。deny 仍不能被同 rank allow 覆盖。
                let allow = r.effect == AccessEffect::Allow;
                match winner {
                    None => winner = Some((rank, allow)),
                    Some((old, _)) if rank > old => winner = Some((rank, allow)),
                    Some((old, previous)) if rank == old => {
                        winner = Some((rank, previous && allow))
                    }
                    _ => {}
                }
            }
        }
        winner.is_none_or(|(_, allow)| allow)
    }
    pub(super) fn path_allows(
        &self,
        mir: &MirUnit,
        path: MirManeuverPathKey,
        class: MirParticipantClassKey,
    ) -> bool {
        if self.entries.is_empty() {
            return true;
        }
        self.allows(mir, 1, path.raw(), class)
            && mir.maneuver_path_edges[mir.maneuver_paths[path.index()].edges.as_usize_range()]
                .iter()
                .all(|edge| self.allows(mir, 0, edge.target.raw(), class))
    }
}

fn visit(mir: &MirUnit, mut emit: impl FnMut(Entry)) {
    for (index, rule) in mir.access_rules.iter().enumerate() {
        let mut edges = |range: crate::arena::TableRange<super::super::MirAuthoringLaneEdge>,
                         specificity| {
            for edge in &mir.authoring_lane_edges[range.as_usize_range()] {
                emit(Entry {
                    plane: 0,
                    target: edge.target.raw(),
                    rule: index as u32,
                    specificity,
                });
            }
        };
        match rule.target {
            super::super::MirAccessTarget::LaneEdge(edge) => emit(Entry {
                plane: 0,
                target: edge.raw(),
                rule: index as u32,
                specificity: 2,
            }),
            super::super::MirAccessTarget::ManeuverPath(path) => emit(Entry {
                plane: 1,
                target: path.raw(),
                rule: index as u32,
                specificity: 0,
            }),
            super::super::MirAccessTarget::LaneGroup(group) => {
                for member in
                    &mir.lane_group_members[mir.lane_groups[group.index()].members.as_usize_range()]
                {
                    edges(mir.authoring_lanes[member.lane.index()].edge_chain, 1);
                }
            }
            super::super::MirAccessTarget::RoadSection(section) => {
                for lane in
                    &mir.authoring_lanes[mir.road_sections[section.index()].lanes.as_usize_range()]
                {
                    edges(lane.edge_chain, 0);
                }
            }
        }
    }
}
