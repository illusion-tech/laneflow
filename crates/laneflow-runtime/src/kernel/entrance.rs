//! 新鲜生成和替换的开放入口与范围内车身。恢复和修订切换不调用。

use laneflow_static_contract::LaneEdgeOrdinal;

use super::input::{EntranceDirection, VehicleEntrance};
use crate::EntranceBodyError;

/// 从当前前保险杠沿路线往回量车长后，仍没有写进路线的毫米数。
/// 下标或边长缺失时返回 `None`。
pub(crate) fn uncovered_tail_mm(
    lengths: &[u32],
    edges: &[LaneEdgeOrdinal],
    mut index: usize,
    mut end: u32,
    mut remaining: u32,
) -> Option<u32> {
    while remaining > 0 {
        let edge = *edges.get(index)?;
        let edge_length = *lengths.get(edge.index())?;
        let start = end.saturating_sub(remaining);
        let hi = end.min(edge_length);
        remaining = remaining.saturating_sub(hi.saturating_sub(start));
        if remaining == 0 || index == 0 {
            break;
        }
        index -= 1;
        end = *lengths.get(edges.get(index)?.index())?;
    }
    Some(remaining)
}

/// 绑定是否在形状上指向这次路线的起点，并且顺着边进入。
fn binding_is_well_formed(entrance: VehicleEntrance) -> bool {
    entrance.route_edge_index() == 0 && entrance.direction() == EntranceDirection::AlongLane
}

impl crate::kernel::state::WorldState {
    /// 车身超出路线起点时，只允许合法开放入口省略范围外的那一段。
    ///
    /// 已建模的车道前驱或机动转移前驱上的车尾必须由调用方写进路线。本函数不选择前驱。
    pub(crate) fn admit_entrance_body(
        &mut self,
        input: crate::VehicleSpawnInput,
        vehicle_length_mm: u32,
    ) -> Result<(), crate::SpawnError> {
        if let Some(entrance) = input.entrance()
            && !binding_is_well_formed(entrance)
        {
            return Err(crate::SpawnError::EntranceBody(
                EntranceBodyError::InvalidBinding,
            ));
        }
        let edges = self
            .route_edges(input.route())
            .ok_or(crate::SpawnError::UnknownRoute)?;
        let cursor = usize::try_from(input.route_edge_index())
            .map_err(|_| crate::SpawnError::InvalidProgress)?;
        let uncovered = uncovered_tail_mm(
            self.binding.revision.traffic().lane_lengths_millimetres(),
            edges,
            cursor,
            input.progress_mm(),
            vehicle_length_mm,
        )
        .ok_or(crate::SpawnError::InvalidProgress)?;
        if uncovered == 0 {
            return Ok(());
        }
        let first = *edges.first().ok_or(crate::SpawnError::UnknownRoute)?;
        if self.route_start_has_upstream(first)? {
            return Err(crate::SpawnError::EntranceBody(
                EntranceBodyError::InDomainTail,
            ));
        }
        if input.entrance().is_some() {
            Ok(())
        } else {
            Err(crate::SpawnError::EntranceBody(EntranceBodyError::Unbound))
        }
    }

    fn route_start_has_upstream(
        &mut self,
        edge: LaneEdgeOrdinal,
    ) -> Result<bool, crate::SpawnError> {
        let traffic = self.binding.revision.traffic();
        if traffic
            .predecessors(edge)
            .is_some_and(|predecessors| !predecessors.is_empty())
        {
            return Ok(true);
        }
        self.workspace
            .occupancy_scratch
            .ensure_maneuver_upstream(traffic)
            .map_err(|_| crate::SpawnError::OccupancyAllocFailed)?;
        Ok(!self
            .workspace
            .occupancy_scratch
            .maneuver_upstream(edge)
            .is_empty())
    }
}

pub(crate) fn entrance_replace_error(error: crate::SpawnError) -> crate::ReplaceError {
    match error {
        crate::SpawnError::EntranceBody(reason) => crate::ReplaceError::EntranceBody(reason),
        crate::SpawnError::UnknownRoute => crate::ReplaceError::UnknownRoute,
        crate::SpawnError::InvalidProgress => crate::ReplaceError::InvalidProgress,
        crate::SpawnError::OccupancyAllocFailed => crate::ReplaceError::OccupancyAllocFailed,
        other => panic!("entrance admission returned {other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::uncovered_tail_mm;
    use crate::kernel::tables::for_each_occupancy_interval;
    use laneflow_static_contract::LaneEdgeOrdinal;

    fn edge(raw: u32) -> LaneEdgeOrdinal {
        LaneEdgeOrdinal::from_raw(raw)
    }

    #[test]
    fn uncovered_tail_matches_the_occupancy_walk() {
        let lengths = [20_000, 10_000, 30_000];
        let edges = [edge(0), edge(1), edge(2)];
        let mut covered = 0_u32;
        for_each_occupancy_interval(&lengths, &edges, 2, 1_000, 4_500, |_, lo, hi| {
            covered += hi - lo;
        })
        .expect("walk");
        assert_eq!(
            uncovered_tail_mm(&lengths, &edges, 2, 1_000, 4_500),
            Some(4_500 - covered)
        );
        assert_eq!(
            uncovered_tail_mm(&lengths, &edges, 0, 2_000, 4_500),
            Some(2_500)
        );
        assert_eq!(
            uncovered_tail_mm(&lengths, &edges, 0, 4_500, 4_500),
            Some(0)
        );
    }
}
