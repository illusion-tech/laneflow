//! 互不可见的共横向过渡段判定（#1220 能力限制，#1321 收紧）。
//!
//! 现行过渡段模型按边身份划分资源格，P4 只能发现路线经过本过渡段或其邻接边
//! 的车辆，P5 的预约停车边界同样只按当前过渡边的直接邻接计算。共横向的两条
//! 过渡（独立分量）只有能证明允许路线分支受现行 P4/P5 共同覆盖时才可同时
//! 存在于路线中；无法证明即结构化拒绝，判定细则（拒绝优先级级联）见
//! `is_invisible_co_lateral_with`，与最小换道合同 §6 第 6 项同口径。退役
//! 条件见合同 §10。
//!
//! 判定对象是独立过渡分量（多段链合成一条），运行时保护单位是成员边；判定
//! 只在路线编译时对含过渡段的路线运行，不保存状态；成本为路线上每条过渡段
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
    /// 成员边的全部前驱（去掉链内成员），已排序去重；边界语义为边终点。
    entries: Vec<LaneEdgeOrdinal>,
    /// 成员边的全部后继（去掉链内成员），已排序去重；边界语义为边起点。
    exits: Vec<LaneEdgeOrdinal>,
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
        let mut entries = Vec::new();
        let mut exits = Vec::new();
        let mut corridor = None;
        let mut lateral_span: Option<((usize, usize), (usize, usize))> = None;
        for member in &members {
            for neighbor in adjacent_edges(traffic, *member) {
                neighborhood.push(neighbor);
                if is_transition_material(neighbor) {
                    continue;
                }
                if traffic
                    .predecessors(*member)
                    .unwrap_or_default()
                    .contains(&neighbor)
                {
                    entries.push(neighbor);
                }
                if traffic
                    .successors(*member)
                    .unwrap_or_default()
                    .contains(&neighbor)
                {
                    exits.push(neighbor);
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
        entries.sort_unstable();
        entries.dedup();
        exits.sort_unstable();
        exits.dedup();
        Some(Self {
            members,
            corridor: corridor?,
            lateral_span: lateral_span?,
            neighborhood,
            entries,
            exits,
        })
    }

    /// 拒绝优先级级联（#1321）：返回 `true` 表示这对共横向分量无法以现行
    /// P4/P5 证明互相覆盖，含任一方的路线应被结构化拒绝。逐条终决、不回落：
    ///
    /// 1. 非共横向（异走廊或横向跨度不相交）：本规则不拒绝。
    /// 2. 邻域完全不相交（#1220 原边界）：互相找不到任何邻接，拒绝。
    /// 3. 共享前驱（分岔）：已进入对方过渡段、位于共享入口附近的车对现行
    ///    P4/P5 不可见——出口侧的停止距离证明保护不到入口重叠区，无证明
    ///    手段；优先于汇合判定（出口全同也拒）。
    /// 4. 任一分量非单成员：运行时保护单位是成员边，分量级出口关系不能
    ///    替代逐成员覆盖证明（首段可在共享出口之前交叉），保守拒绝；孤立
    ///    多段链没有配对，不进入本判定。
    /// 5. 共享后继（汇合）：仅共同唯一出口放行——直接后继使 P4 证明边界
    ///    与 P5 停车边界一致（出口起点）；唯一性使两份过渡预约的下游空间
    ///    都覆盖该出口的非空前缀、必然相交，由共同资源仲裁串行。集合相等
    ///    但有多个出口会失去预约互斥（各选其一循环互等），直接拒绝、
    ///    不回落接力。
    /// 6. 接力：仅单锚必经放行——一方唯一出口恰为另一方唯一入口，能获准
    ///    准入并实际穿越、清空的执行路径无法绕过该边（无下游净空的路线
    ///    另受既有净空检查约束）：进入者在共享边上时区间内必拦，接近时
    ///    边界取该边起点或终点；存在绕过分支即拒绝。
    fn is_invisible_co_lateral_with(&self, other: &Self) -> bool {
        if !(self.corridor == other.corridor
            && self.lateral_span.0 <= other.lateral_span.1
            && other.lateral_span.0 <= self.lateral_span.1)
        {
            return false;
        }
        if !sorted_slices_intersect(&self.neighborhood, &other.neighborhood) {
            return true;
        }
        if sorted_slices_intersect(&self.entries, &other.entries) {
            return true;
        }
        if self.members.len() > 1 || other.members.len() > 1 {
            return true;
        }
        if sorted_slices_intersect(&self.exits, &other.exits) {
            return !(self.exits.len() == 1 && self.exits == other.exits);
        }
        let relayed = |upstream: &Self, downstream: &Self| {
            upstream.exits.len() == 1
                && downstream.entries.len() == 1
                && upstream.exits == downstream.entries
        };
        !(relayed(self, other) || relayed(other, self))
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
    // 按边序号索引：过渡段互不相邻时，线性查找会让一次判定退化为 O(T²)。
    let mut visited = vec![false; traffic.lane_edge_count() as usize];
    mark_visited(&mut visited, &own.members);
    for raw in 0..traffic.lane_edge_count() {
        let edge = LaneEdgeOrdinal::from_raw(raw);
        if !traffic.is_lateral_transition_segment(edge) || visited[edge.index()] {
            continue;
        }
        let Some(other) = TransitionFootprint::collect(traffic, edge) else {
            return true;
        };
        if own.is_invisible_co_lateral_with(&other) {
            return true;
        }
        mark_visited(&mut visited, &other.members);
    }
    false
}

fn mark_visited(visited: &mut [bool], members: &[LaneEdgeOrdinal]) {
    for member in members {
        visited[member.index()] = true;
    }
}
