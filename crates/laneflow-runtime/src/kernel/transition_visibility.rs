//! 互不可见的共横向过渡段判定（#1220 能力限制）。
//!
//! 现行过渡段模型按边身份划分资源格，P4 只能发现路线经过本过渡段或其邻接边的
//! 车辆。两条过渡段横向相交而邻接边不相交时，双方都可能获得授予，因此拒绝含这种
//! 过渡段的路线。退役条件见最小换道合同 §10。
//!
//! 判定只在路线编译时对含过渡段的路线运行，不保存状态；成本为路线上每条过渡段
//! 扫描一次全部车道边。

use laneflow_static_contract::LaneEdgeOrdinal;
use laneflow_static_network::{CorridorElement, SharedRelationClosure, SharedTrafficNetwork};

/// 一条过渡（含多段过渡链）的判定输入。
struct TransitionFootprint {
    members: Vec<LaneEdgeOrdinal>,
    corridor: u32,
    /// 横向坐标（走廊元素位置，区段内车道序数）的最小值与最大值。
    lateral_span: ((usize, usize), (usize, usize)),
    /// 过渡自身及其全部前驱、后继边，已排序去重。
    neighborhood: Vec<LaneEdgeOrdinal>,
}

impl TransitionFootprint {
    fn collect(traffic: &SharedTrafficNetwork, transition: LaneEdgeOrdinal) -> Option<Self> {
        let relations = traffic.relations();
        let is_transition_material = |edge: LaneEdgeOrdinal| {
            relations.lane_edge_authoring_lane(edge).is_none()
                && relations.lane_edge_junction(edge).is_none()
        };
        let mut members = vec![transition];
        let mut cursor = 0;
        while let Some(&member) = members.get(cursor) {
            cursor += 1;
            for neighbor in adjacent_edges(traffic, member) {
                if is_transition_material(neighbor) && !members.contains(&neighbor) {
                    members.push(neighbor);
                }
            }
        }
        let mut neighborhood = members.clone();
        let mut corridor = None;
        let mut lateral_span: Option<((usize, usize), (usize, usize))> = None;
        for member in &members {
            for neighbor in adjacent_edges(traffic, *member) {
                neighborhood.push(neighbor);
                if is_transition_material(neighbor) {
                    continue;
                }
                let Some((boundary_corridor, coordinate)) = lateral_position(relations, neighbor)
                else {
                    continue;
                };
                corridor = Some(boundary_corridor);
                lateral_span = Some(lateral_span.map_or((coordinate, coordinate), |(lo, hi)| {
                    (lo.min(coordinate), hi.max(coordinate))
                }));
            }
        }
        neighborhood.sort_unstable();
        neighborhood.dedup();
        Some(Self {
            members,
            corridor: corridor?,
            lateral_span: lateral_span?,
            neighborhood,
        })
    }

    fn is_invisible_co_lateral_with(&self, other: &Self) -> bool {
        self.corridor == other.corridor
            && self.lateral_span.0 <= other.lateral_span.1
            && other.lateral_span.0 <= self.lateral_span.1
            && !sorted_slices_intersect(&self.neighborhood, &other.neighborhood)
    }
}

fn adjacent_edges(
    traffic: &SharedTrafficNetwork,
    edge: LaneEdgeOrdinal,
) -> impl Iterator<Item = LaneEdgeOrdinal> + '_ {
    traffic
        .successors(edge)
        .unwrap_or_default()
        .iter()
        .chain(traffic.predecessors(edge).unwrap_or_default())
        .copied()
}

/// 与共享静态路网的横向过渡段分类使用同一组横向坐标。
fn lateral_position(
    relations: &SharedRelationClosure,
    edge: LaneEdgeOrdinal,
) -> Option<(u32, (usize, usize))> {
    let lane = relations.lane_edge_authoring_lane(edge)?;
    let section = relations.authoring_section(lane)?;
    let corridor = relations.section_corridor(section)?;
    let element = relations
        .corridor_elements(corridor)?
        .iter()
        .position(|element| *element == CorridorElement::RoadSection(section))?;
    let lane_index = relations
        .section_lanes(section)?
        .iter()
        .position(|member| *member == lane)?;
    Some((corridor.raw(), (element, lane_index)))
}

fn sorted_slices_intersect(left: &[LaneEdgeOrdinal], right: &[LaneEdgeOrdinal]) -> bool {
    let (mut l, mut r) = (0, 0);
    while let (Some(a), Some(b)) = (left.get(l), right.get(r)) {
        match a.cmp(b) {
            std::cmp::Ordering::Less => l += 1,
            std::cmp::Ordering::Greater => r += 1,
            std::cmp::Ordering::Equal => return true,
        }
    }
    false
}

/// 路网中是否存在与 `transition` 互不可见的共横向过渡段。
///
/// 输入不是可判定的横向过渡段时返回 `true`：调用方据此拒绝路线，不静默放行。
pub(crate) fn has_invisible_co_lateral_transition(
    traffic: &SharedTrafficNetwork,
    transition: LaneEdgeOrdinal,
) -> bool {
    let Some(own) = TransitionFootprint::collect(traffic, transition) else {
        return true;
    };
    let mut visited = own.members.clone();
    for raw in 0..traffic.lane_edge_count() {
        let edge = LaneEdgeOrdinal::from_raw(raw);
        if !traffic.is_lateral_transition_segment(edge) || visited.contains(&edge) {
            continue;
        }
        let Some(other) = TransitionFootprint::collect(traffic, edge) else {
            return true;
        };
        if own.is_invisible_co_lateral_with(&other) {
            return true;
        }
        visited.extend_from_slice(&other.members);
    }
    false
}
