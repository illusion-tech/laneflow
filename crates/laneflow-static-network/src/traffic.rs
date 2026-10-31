use core::mem::size_of;

use laneflow_static_contract::{
    EntityKind, LaneEdgeOrdinal, ManeuverGateOrdinal, ManeuverPathOrdinal, MovementOrdinal,
    WaitingZoneOrdinal,
};

use crate::{BuildError, BuildStructure};

const ENTITY_KIND_COUNT: usize = EntityKind::ALL.len();

/// 连续 flat payload 中的受检 `u32` 区间。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RangeU32 {
    pub(crate) start: u32,
    pub(crate) len: u32,
}

impl RangeU32 {
    /// 由起始下标与长度构造区间。
    pub(crate) const fn new(start: u32, len: u32) -> Self {
        Self { start, len }
    }

    /// 区间起始下标。
    #[must_use]
    pub const fn start(self) -> u32 {
        self.start
    }

    /// 区间包含的元素个数。
    #[must_use]
    pub const fn len(self) -> u32 {
        self.len
    }

    /// 区间是否不含任何元素。
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    /// 按本区间从连续切片中借用对应子切片。
    pub(crate) fn slice<T>(self, values: &[T]) -> &[T] {
        let start = usize::try_from(self.start).expect("checked u32 range start");
        let end = usize::try_from(
            self.start
                .checked_add(self.len)
                .expect("checked u32 range end"),
        )
        .expect("checked u32 range end fits usize");
        &values[start..end]
    }
}

/// 23 个可构造实体种类的致密表基数。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EntityCounts {
    pub(crate) counts: [u32; ENTITY_KIND_COUNT],
}

/// 一条 Runtime 可执行 transition 对应的机动路径上下文。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManeuverTransitionCandidate {
    pub(crate) successor: LaneEdgeOrdinal,
    pub(crate) maneuver_path: ManeuverPathOrdinal,
    pub(crate) transition_index: u32,
    pub(crate) maneuver_gate: Option<ManeuverGateOrdinal>,
}

impl ManeuverTransitionCandidate {
    /// 组装一条 transition candidate 的全部上下文字段。
    pub(crate) const fn new(
        successor: LaneEdgeOrdinal,
        maneuver_path: ManeuverPathOrdinal,
        transition_index: u32,
        maneuver_gate: Option<ManeuverGateOrdinal>,
    ) -> Self {
        Self {
            successor,
            maneuver_path,
            transition_index,
            maneuver_gate,
        }
    }

    /// transition 指向的后继 LaneEdge。
    #[must_use]
    pub const fn successor(self) -> LaneEdgeOrdinal {
        self.successor
    }

    /// 该 transition 所属的机动路径。
    #[must_use]
    pub const fn maneuver_path(self) -> ManeuverPathOrdinal {
        self.maneuver_path
    }

    /// 该 transition 在机动路径内的下标。
    #[must_use]
    pub const fn transition_index(self) -> u32 {
        self.transition_index
    }

    /// 关联的机动门（若该 transition 受门约束）。
    #[must_use]
    pub const fn maneuver_gate(self) -> Option<ManeuverGateOrdinal> {
        self.maneuver_gate
    }
}

/// 一条规范机动路径的连续 Runtime 借用。
#[derive(Clone, Copy, Debug)]
pub struct ManeuverPathView<'a> {
    movement: MovementOrdinal,
    edges: &'a [LaneEdgeOrdinal],
    maneuver_gates: &'a [ManeuverGateOrdinal],
    waiting_zones: &'a [WaitingZoneOrdinal],
}

impl<'a> ManeuverPathView<'a> {
    /// 该机动路径所属的通行流向。
    #[must_use]
    pub const fn movement(self) -> MovementOrdinal {
        self.movement
    }

    /// 路径依次遍历的 LaneEdge 序列。
    #[must_use]
    pub const fn edges(self) -> &'a [LaneEdgeOrdinal] {
        self.edges
    }

    /// 路径上绑定的机动门序列。
    #[must_use]
    pub const fn maneuver_gates(self) -> &'a [ManeuverGateOrdinal] {
        self.maneuver_gates
    }

    /// 路径上绑定的等待区序列。
    #[must_use]
    pub const fn waiting_zones(self) -> &'a [WaitingZoneOrdinal] {
        self.waiting_zones
    }
}

/// 路口机动路径、transition candidate 与 gate/waiting 引用的共享静态数据。
pub struct SharedManeuverNetwork {
    pub(crate) movements: Box<[MovementOrdinal]>,
    pub(crate) edge_ranges: Box<[RangeU32]>,
    pub(crate) edges: Box<[LaneEdgeOrdinal]>,
    pub(crate) maneuver_gate_ranges: Box<[RangeU32]>,
    pub(crate) maneuver_gates: Box<[ManeuverGateOrdinal]>,
    pub(crate) waiting_zone_ranges: Box<[RangeU32]>,
    pub(crate) waiting_zones: Box<[WaitingZoneOrdinal]>,
    pub(crate) candidate_ranges: Box<[RangeU32]>,
    pub(crate) candidates: Box<[ManeuverTransitionCandidate]>,
}

impl SharedManeuverNetwork {
    /// 由预构建的各连续 payload 组装共享机动数据。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        movements: Box<[MovementOrdinal]>,
        edge_ranges: Box<[RangeU32]>,
        edges: Box<[LaneEdgeOrdinal]>,
        maneuver_gate_ranges: Box<[RangeU32]>,
        maneuver_gates: Box<[ManeuverGateOrdinal]>,
        waiting_zone_ranges: Box<[RangeU32]>,
        waiting_zones: Box<[WaitingZoneOrdinal]>,
        candidate_ranges: Box<[RangeU32]>,
        candidates: Box<[ManeuverTransitionCandidate]>,
    ) -> Self {
        Self {
            movements,
            edge_ranges,
            edges,
            maneuver_gate_ranges,
            maneuver_gates,
            waiting_zone_ranges,
            waiting_zones,
            candidate_ranges,
            candidates,
        }
    }

    /// 机动路径总数。
    #[must_use]
    pub fn maneuver_path_count(&self) -> u32 {
        u32::try_from(self.movements.len()).expect("format-bounded maneuver path count fits u32")
    }

    /// 按 ordinal 借用一条机动路径的只读视图。
    #[must_use]
    pub fn maneuver_path(&self, path: ManeuverPathOrdinal) -> Option<ManeuverPathView<'_>> {
        let index = path.index();
        Some(ManeuverPathView {
            movement: *self.movements.get(index)?,
            edges: self.edge_ranges.get(index)?.slice(&self.edges),
            maneuver_gates: self
                .maneuver_gate_ranges
                .get(index)?
                .slice(&self.maneuver_gates),
            waiting_zones: self
                .waiting_zone_ranges
                .get(index)?
                .slice(&self.waiting_zones),
        })
    }

    /// 各机动路径的机动门区间表。
    pub(crate) fn path_gate_ranges(&self) -> &[RangeU32] {
        &self.maneuver_gate_ranges
    }

    /// 全部机动路径共享的机动门 flat payload。
    pub(crate) fn path_gates(&self) -> &[ManeuverGateOrdinal] {
        &self.maneuver_gates
    }

    /// 各机动路径的等待区区间表。
    pub(crate) fn path_waiting_ranges(&self) -> &[RangeU32] {
        &self.waiting_zone_ranges
    }

    /// 全部机动路径共享的等待区 flat payload。
    pub(crate) fn path_waiting_zones(&self) -> &[WaitingZoneOrdinal] {
        &self.waiting_zones
    }

    /// 查询一条前驱 LaneEdge 的全部可执行 transition candidate。
    #[must_use]
    pub fn transition_candidates(
        &self,
        predecessor: LaneEdgeOrdinal,
    ) -> Option<&[ManeuverTransitionCandidate]> {
        let range = *self.candidate_ranges.get(predecessor.index())?;
        Some(range.slice(&self.candidates))
    }

    /// 本结构保留的逻辑字节数（含全部连续 payload）。
    #[must_use]
    pub fn retained_logical_bytes(&self) -> u64 {
        logical_bytes::<MovementOrdinal>(self.movements.len())
            + logical_bytes::<RangeU32>(self.edge_ranges.len())
            + logical_bytes::<LaneEdgeOrdinal>(self.edges.len())
            + logical_bytes::<RangeU32>(self.maneuver_gate_ranges.len())
            + logical_bytes::<ManeuverGateOrdinal>(self.maneuver_gates.len())
            + logical_bytes::<RangeU32>(self.waiting_zone_ranges.len())
            + logical_bytes::<WaitingZoneOrdinal>(self.waiting_zones.len())
            + logical_bytes::<RangeU32>(self.candidate_ranges.len())
            + logical_bytes::<ManeuverTransitionCandidate>(self.candidates.len())
    }
}

impl EntityCounts {
    /// 由按实体种类排列的基数数组构造。
    pub(crate) const fn new(counts: [u32; ENTITY_KIND_COUNT]) -> Self {
        Self { counts }
    }

    /// 指定实体种类的实例基数。
    #[must_use]
    pub const fn count(self, entity_kind: EntityKind) -> u32 {
        self.counts[(entity_kind.code() - 1) as usize]
    }

    /// 按标记类型查询对应实体种类的实例基数。
    #[must_use]
    pub fn typed_count<K: laneflow_static_contract::EntityKindMarker>(&self) -> u32 {
        self.count(K::KIND)
    }

    /// 以数组形式借用全部实体种类基数。
    #[must_use]
    pub const fn as_array(&self) -> &[u32; ENTITY_KIND_COUNT] {
        &self.counts
    }
}

/// 面向 Traffic Runtime 的必选共享静态数据。
///
/// 当前基线冻结所有实体基数、LaneEdge 热列/完整可执行 CSR，以及携带路径、transition、
/// gate/waiting 引用的路口机动候选；后继 #440 实现切片继续加入其余静态关系，但不会改变
/// 根对象唯一共享所有权。
pub struct SharedTrafficNetwork {
    pub(crate) entity_counts: EntityCounts,
    pub(crate) lane_lengths_millimetres: Box<[u32]>,
    pub(crate) lane_speed_limits_millimetres_per_second: Box<[u32]>,
    pub(crate) successor_ranges: Box<[RangeU32]>,
    pub(crate) successors: Box<[LaneEdgeOrdinal]>,
    pub(crate) predecessor_ranges: Box<[RangeU32]>,
    pub(crate) predecessors: Box<[LaneEdgeOrdinal]>,
    pub(crate) maneuvers: SharedManeuverNetwork,
    pub(crate) relations: crate::SharedRelationClosure,
    /// 横向过渡段派生分类位集（#1194）：构建期一次计算，注册期查表。
    pub(crate) lateral_transition_segments: Box<[u64]>,
}

/// 横向过渡段派生分类（#1194；判据整体重设计——接缝矩阵与正反例测试见运行时
/// `compile_route_tests::s3_lane_change::p0_transition_classification_matrix`）：
/// 构建期一次计算（分量单遍分类），注册期 O(1) 查表。
///
/// 判定谓词：`candidate` 未被 lane 覆盖且非 junction internal 时，求其
/// uncovered 连通分量（后继前向 + 前驱后向）；分量两侧 covered 边界的
/// **横向坐标** =（其 section 在走廊 elements 中的横断面位置，section 内
/// lane 横向序数）；**同走廊 且 横向坐标跨度 ≥ 2 → 横向过渡段**。
///
/// 接缝矩阵（正=过渡段，负=非过渡）：异 lane 横移（同 section 不同 lane
/// 序数）→ 正；跨 section 横向接缝（相邻单 lane section 元素位不同）→ 正
/// （P1 补全）；多段链 / 起止于过渡 → 正（段级分类，与路线窗口无关）；
/// 纵向连接（两侧不同走廊）→ 负（P2-2 排除）；同横向坐标续接 → 负。
fn compute_lateral_transition_segments(
    successor_ranges: &[RangeU32],
    successors: &[LaneEdgeOrdinal],
    predecessor_ranges: &[RangeU32],
    predecessors: &[LaneEdgeOrdinal],
    relations: &crate::SharedRelationClosure,
    edge_count: u32,
) -> Box<[u64]> {
    let uncovered_non_junction = |edge: LaneEdgeOrdinal| {
        relations.lane_edge_authoring_lane(edge).is_none()
            && relations.lane_edge_junction(edge).is_none()
    };
    // 横向坐标 =（section 在走廊 elements 中的横断面位置，section 内 lane 序数）。
    let lateral_coordinate = |edge: LaneEdgeOrdinal| -> Option<(usize, usize)> {
        let lane = relations.lane_edge_authoring_lane(edge)?;
        let section = relations.authoring_section(lane)?;
        let corridor = relations.section_corridor(section)?;
        let elements = relations.corridor_elements(corridor)?;
        let element_position = elements
            .iter()
            .position(|element| *element == crate::CorridorElement::RoadSection(section))?;
        let lanes = relations.section_lanes(section)?;
        let lane_index = lanes.iter().position(|member| *member == lane)?;
        Some((element_position, lane_index))
    };
    let successors_of = |edge: LaneEdgeOrdinal| -> &[LaneEdgeOrdinal] {
        successor_ranges
            .get(edge.index())
            .map(|range| range.slice(successors))
            .unwrap_or_default()
    };
    let predecessors_of = |edge: LaneEdgeOrdinal| -> &[LaneEdgeOrdinal] {
        predecessor_ranges
            .get(edge.index())
            .map(|range| range.slice(predecessors))
            .unwrap_or_default()
    };
    let capacity = edge_count as usize;
    let mut words = vec![0_u64; capacity.div_ceil(64)];
    // 分量单遍分类（P2 性能）：processed 兼任 visited 与完成态，每个连通
    // 分量只求解并分类一次，无逐候选重扫。
    let mut processed = vec![false; capacity];
    for raw in 0..edge_count {
        let candidate = LaneEdgeOrdinal::from_raw(raw);
        if !uncovered_non_junction(candidate) || processed[candidate.index()] {
            continue;
        }
        let mut component = vec![candidate];
        processed[candidate.index()] = true;
        let mut cursor = 0;
        while cursor < component.len() {
            let edge = component[cursor];
            cursor += 1;
            for successor in successors_of(edge) {
                if uncovered_non_junction(*successor) && !processed[successor.index()] {
                    processed[successor.index()] = true;
                    component.push(*successor);
                }
            }
            for predecessor in predecessors_of(edge) {
                if uncovered_non_junction(*predecessor) && !processed[predecessor.index()] {
                    processed[predecessor.index()] = true;
                    component.push(*predecessor);
                }
            }
        }
        // 分量两侧 covered 边界的走廊集合与横向坐标集合。
        let mut corridors: std::collections::HashSet<u32> = std::collections::HashSet::new();
        let mut coordinates: std::collections::HashSet<(usize, usize)> =
            std::collections::HashSet::new();
        for member in component.clone() {
            let mut boundaries = successors_of(member).to_vec();
            boundaries.extend_from_slice(predecessors_of(member));
            for boundary in boundaries {
                if uncovered_non_junction(boundary) {
                    continue;
                }
                let Some(coordinate) = lateral_coordinate(boundary) else {
                    continue;
                };
                coordinates.insert(coordinate);
                let Some(lane) = relations.lane_edge_authoring_lane(boundary) else {
                    continue;
                };
                let Some(section) = relations.authoring_section(lane) else {
                    continue;
                };
                if let Some(corridor) = relations.section_corridor(section) {
                    corridors.insert(corridor.raw());
                }
            }
        }
        if corridors.len() == 1 && coordinates.len() >= 2 {
            for member in &component {
                words[member.index() / 64] |= 1_u64 << (member.index() % 64);
            }
        }
    }
    words.into_boxed_slice()
}

impl SharedTrafficNetwork {
    /// 由预构建的实体基数、车道列与机动/关系数据组装共享交通路网。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        entity_counts: EntityCounts,
        lane_lengths_millimetres: Box<[u32]>,
        lane_speed_limits_millimetres_per_second: Box<[u32]>,
        successor_ranges: Box<[RangeU32]>,
        successors: Box<[LaneEdgeOrdinal]>,
        predecessor_ranges: Box<[RangeU32]>,
        predecessors: Box<[LaneEdgeOrdinal]>,
        maneuvers: SharedManeuverNetwork,
        relations: crate::SharedRelationClosure,
    ) -> Self {
        let lateral_transition_segments = compute_lateral_transition_segments(
            &successor_ranges,
            &successors,
            &predecessor_ranges,
            &predecessors,
            &relations,
            entity_counts.count(EntityKind::LaneEdge),
        );
        Self {
            entity_counts,
            lane_lengths_millimetres,
            lane_speed_limits_millimetres_per_second,
            successor_ranges,
            successors,
            predecessor_ranges,
            predecessors,
            maneuvers,
            relations,
            lateral_transition_segments,
        }
    }

    /// 横向过渡段（固定预制过渡，如 `T01`）派生分类查表（#1194）：构建期
    /// 一次计算（`compute_lateral_transition_segments`），注册期 O(1) 查询。
    #[must_use]
    pub fn is_lateral_transition_segment(&self, lane_edge: LaneEdgeOrdinal) -> bool {
        let raw = lane_edge.index();
        let Some(word) = self.lateral_transition_segments.get(raw / 64) else {
            return false;
        };
        (word >> (raw % 64)) & 1 == 1
    }

    /// 全部实体种类的基数表。
    #[must_use]
    pub const fn entity_counts(&self) -> &EntityCounts {
        &self.entity_counts
    }

    /// LaneEdge 总数。
    #[must_use]
    pub fn lane_edge_count(&self) -> u32 {
        self.entity_counts.count(EntityKind::LaneEdge)
    }

    /// 各 LaneEdge 的长度列（整数毫米）。
    #[must_use]
    pub fn lane_lengths_millimetres(&self) -> &[u32] {
        &self.lane_lengths_millimetres
    }

    /// 各 LaneEdge 的速度上限列（整数毫米/秒）。
    #[must_use]
    pub fn lane_speed_limits_millimetres_per_second(&self) -> &[u32] {
        &self.lane_speed_limits_millimetres_per_second
    }

    /// 一条 LaneEdge 的全部后继 LaneEdge。
    #[must_use]
    pub fn successors(&self, lane_edge: LaneEdgeOrdinal) -> Option<&[LaneEdgeOrdinal]> {
        let range = *self.successor_ranges.get(lane_edge.index())?;
        Some(range.slice(&self.successors))
    }

    /// 一条 LaneEdge 的全部前驱 LaneEdge。
    #[must_use]
    pub fn predecessors(&self, lane_edge: LaneEdgeOrdinal) -> Option<&[LaneEdgeOrdinal]> {
        let range = *self.predecessor_ranges.get(lane_edge.index())?;
        Some(range.slice(&self.predecessors))
    }

    /// 路口机动路径与 transition candidate 的共享数据。
    #[must_use]
    pub const fn maneuvers(&self) -> &SharedManeuverNetwork {
        &self.maneuvers
    }

    /// 共享关系闭包。
    #[must_use]
    pub const fn relations(&self) -> &crate::SharedRelationClosure {
        &self.relations
    }

    /// 本共享交通路网保留的逻辑字节数（含机动与关系数据）。
    #[must_use]
    pub fn retained_logical_bytes(&self) -> u64 {
        logical_bytes::<u32>(self.lane_lengths_millimetres.len())
            + logical_bytes::<u32>(self.lane_speed_limits_millimetres_per_second.len())
            + logical_bytes::<RangeU32>(self.successor_ranges.len())
            + logical_bytes::<LaneEdgeOrdinal>(self.successors.len())
            + logical_bytes::<RangeU32>(self.predecessor_ranges.len())
            + logical_bytes::<LaneEdgeOrdinal>(self.predecessors.len())
            + logical_bytes::<u64>(self.lateral_transition_segments.len())
            + self.maneuvers.retained_logical_bytes()
            + self.relations.retained_logical_bytes()
    }
}

/// 从规范 LaneEdge 关系确定性派生、与 worker 数无关的非语义规划提示。
pub struct PartitionPlanningHints {
    pub(crate) edge_boundary_weights: Box<[u32]>,
}

impl PartitionPlanningHints {
    /// 从 `SharedTrafficNetwork` 确定性派生边界权重，可经取消回调中断。
    pub(crate) fn from_traffic(
        traffic: &SharedTrafficNetwork,
        mut poll_cancelled: impl FnMut(u32) -> Result<(), BuildError>,
    ) -> Result<Self, BuildError> {
        poll_cancelled(0)?;
        let capacity =
            usize::try_from(traffic.lane_edge_count()).expect("u32 lane count fits usize");
        let mut edge_boundary_weights = Vec::new();
        edge_boundary_weights
            .try_reserve_exact(capacity)
            .map_err(|_| BuildError::AllocationFailure {
                structure: BuildStructure::PlanningHints,
            })?;
        for raw in 0..traffic.lane_edge_count() {
            if raw != 0 {
                poll_cancelled(raw)?;
            }
            let edge = LaneEdgeOrdinal::from_raw(raw);
            let successors = traffic.successors(edge).map_or(0, <[LaneEdgeOrdinal]>::len);
            let predecessors = traffic
                .predecessors(edge)
                .map_or(0, <[LaneEdgeOrdinal]>::len);
            edge_boundary_weights.push(
                u32::try_from(successors + predecessors)
                    .expect("format-bounded adjacency fits u32"),
            );
        }
        Ok(Self {
            edge_boundary_weights: edge_boundary_weights.into_boxed_slice(),
        })
    }

    /// 各 LaneEdge 的边界权重（前驱与后继度数之和）。
    #[must_use]
    pub fn edge_boundary_weights(&self) -> &[u32] {
        &self.edge_boundary_weights
    }

    /// 本提示保留的逻辑字节数。
    #[must_use]
    pub fn retained_logical_bytes(&self) -> u64 {
        logical_bytes::<u32>(self.edge_boundary_weights.len())
    }
}

/// 计算 `len` 个 `T` 元素占用的逻辑字节数。
pub(crate) fn logical_bytes<T>(len: usize) -> u64 {
    let bytes = len
        .checked_mul(size_of::<T>())
        .expect("format-bounded retained length must fit usize");
    u64::try_from(bytes).expect("retained length must fit u64")
}

#[cfg(test)]
mod tests {
    use core::cell::Cell;

    use super::*;

    #[test]
    fn planning_hints_propagate_cancellation_from_lane_scan() {
        let lane_count = 2_u32;
        let mut entity_counts = [0; ENTITY_KIND_COUNT];
        entity_counts[(EntityKind::LaneEdge.code() - 1) as usize] = lane_count;
        let empty_ranges = || vec![RangeU32::new(0, 0); lane_count as usize].into_boxed_slice();
        let maneuvers = SharedManeuverNetwork::new(
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            empty_ranges(),
            Box::new([]),
        );
        let traffic = SharedTrafficNetwork::new(
            EntityCounts::new(entity_counts),
            vec![1_000; lane_count as usize].into_boxed_slice(),
            vec![1_000; lane_count as usize].into_boxed_slice(),
            empty_ranges(),
            Box::new([]),
            empty_ranges(),
            Box::new([]),
            maneuvers,
            crate::relations::empty_for_tests(lane_count),
        );
        let last_polled = Cell::new(None);

        let result = PartitionPlanningHints::from_traffic(&traffic, |raw| {
            last_polled.set(Some(raw));
            if raw == 1 {
                Err(BuildError::Cancelled)
            } else {
                Ok(())
            }
        });

        assert!(matches!(result, Err(BuildError::Cancelled)));
        assert_eq!(last_polled.get(), Some(1));
    }
}
