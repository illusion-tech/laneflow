use super::{Failure, HandoffBinding, HandoffSourceEvidence, HandoffWindow, lane_pair};
use crate::{
    BoundaryExtent, BoundaryOwner, BoundaryPayload, CorridorElement, RoadSurfaceSpan,
    SharedNetworkRevision, StationQuery,
};
use laneflow_static_contract::{
    AuthoringLaneOrdinal, EntityKindMarker, LaneEdgeOrdinal, Ordinal, OrdinalKind,
};

fn counterpart<K: EntityKindMarker + OrdinalKind>(
    base: &SharedNetworkRevision,
    target: &SharedNetworkRevision,
    old: Ordinal<K>,
) -> Result<Ordinal<K>, Failure> {
    base.identity()
        .stable_id(old)
        .and_then(|id| target.identity().ordinal(id))
        .ok_or(Failure::Binding)
}

fn same_identity<K: EntityKindMarker + OrdinalKind + PartialEq>(
    base: &SharedNetworkRevision,
    target: &SharedNetworkRevision,
    old: Ordinal<K>,
    new: Ordinal<K>,
) -> Result<(), Failure> {
    if counterpart(base, target, old)? == new {
        Ok(())
    } else {
        Err(Failure::Binding)
    }
}

fn project_window(
    network: &SharedNetworkRevision,
    binding: HandoffBinding,
    edge: LaneEdgeOrdinal,
) -> Result<AuthoringLaneOrdinal, Failure> {
    let lane = network
        .traffic()
        .relations()
        .lane_edge_authoring_lane(edge)
        .ok_or(Failure::Binding)?;
    match network.station_mapping().in_station_range(
        binding.corridor(),
        binding.common_stations_nm(),
        lane,
    ) {
        StationQuery::Complete(found)
            if found.addresses().map(|address| address.edge).eq([edge]) =>
        {
            Ok(lane)
        }
        StationQuery::Unproven { .. } => Err(Failure::WorkLimit),
        _ => Err(Failure::Binding),
    }
}

fn target_window(
    base: &SharedNetworkRevision,
    target: &SharedNetworkRevision,
    old: HandoffBinding,
) -> Result<HandoffWindow, Failure> {
    let previous = old.window();
    let [a, b] = previous.edges;
    let edges = [counterpart(base, target, a)?, counterpart(base, target, b)?];
    let r = target.traffic().relations();
    let [left, _] = lane_pair(base, old.corridor(), previous, || Ok(()))?;
    let left = counterpart(base, target, left)?;
    let (owner, index) = match previous.owner {
        BoundaryOwner::Section(s) => {
            let s = counterpart(base, target, s)?;
            (
                BoundaryOwner::Section(s),
                r.section_lanes(s)
                    .and_then(|lanes| lanes.iter().position(|&l| l == left)),
            )
        }
        BoundaryOwner::Corridor(c) => {
            let c = counterpart(base, target, c)?;
            let section = r.authoring_section(left).ok_or(Failure::Binding)?;
            (
                BoundaryOwner::Corridor(c),
                r.corridor_elements(c).and_then(|elements| {
                    elements
                        .iter()
                        .position(|&e| e == CorridorElement::RoadSection(section))
                }),
            )
        }
    };
    let left_index = u32::try_from(index.ok_or(Failure::Binding)?).map_err(|_| Failure::Binding)?;
    Ok(HandoffWindow {
        edges,
        owner,
        left_index,
        stations_mm: previous.stations_mm,
    })
}

fn boundary_rows(network: &SharedNetworkRevision, window: HandoffWindow) -> &[BoundaryExtent] {
    let rows = network.station_mapping().boundary_extents();
    let key = (window.owner, window.left_index);
    let start = rows.partition_point(|r| (r.owner, r.left_index) < key);
    let end = rows.partition_point(|r| (r.owner, r.left_index) <= key);
    &rows[start..end]
}

fn surfaces(network: &SharedNetworkRevision, binding: HandoffBinding) -> &[RoadSurfaceSpan] {
    let rows = network.station_mapping().surface_spans(binding.corridor());
    let [lo, hi] = binding.common_stations_nm();
    let first = rows.partition_point(|r| r.stations_nm()[1].0 < lo);
    let end = rows.partition_point(|r| r.stations_nm()[0].0 <= hi);
    &rows[first..end]
}

fn window_open(
    network: &SharedNetworkRevision,
    binding: HandoffBinding,
    direction: u8,
) -> Result<(), Failure> {
    let rows = boundary_rows(network, binding.window());
    let [lo, hi] = binding.common_stations_nm();
    // 护栏端帽和实际长度可越出登记端点。没有车身候选时，不把仅凭锚点
    // 分开的护栏当作已清空；路缘则以两端站位（含误差）为纵向覆盖域。
    if rows.iter().any(|r| match r.payload {
        Some(BoundaryPayload::Guardrail { .. }) => true,
        Some(BoundaryPayload::Curb { .. }) => {
            r.stations_nm()[0].saturating_sub(r.errors_nm()[0]) <= hi
                && r.stations_nm()[1].saturating_add(r.errors_nm()[1]) >= lo
        }
        None => true,
        _ => false,
    }) {
        return Err(Failure::Unavailable);
    }
    super::super::sweep::window_permission(rows, [lo, hi], direction)
}

impl SharedNetworkRevision {
    /// 对尚未进入的已注册窗口证明两端来自同一编制参考线，并重新验证目标窗口与出现边。
    /// 首版以走廊所在道路编辑文档的完整字节相等证明参考线、原点与站位定义相同；
    /// 走向引用只在该来源模块内解析。其他模块可变更，局部序号可重新排列。
    ///
    /// # Errors
    ///
    /// 任一侧缺少精确来源、相关文档变更、成员或两边实际拥有的站位闭区间改变时拒绝。
    pub fn rebind_handoff_window(
        &self,
        base: &SharedNetworkRevision,
        old: HandoffBinding,
        old_sources: &HandoffSourceEvidence<'_>,
        new_sources: &HandoffSourceEvidence<'_>,
    ) -> Result<HandoffBinding, Failure> {
        if old != base.bind_handoff_window(old.revision(), old.window())? {
            return Err(Failure::Binding);
        }
        let new =
            self.bind_handoff_window(self.network_revision(), target_window(base, self, old)?)?;
        if old.window().stations_mm != new.window().stations_mm
            || old.increasing() != new.increasing()
            || old.common_stations_nm() != new.common_stations_nm()
            || base.station_mapping().corridor_domain(old.corridor())
                != self.station_mapping().corridor_domain(new.corridor())
        {
            return Err(Failure::Binding);
        }
        same_identity(base, self, old.corridor(), new.corridor())?;
        same_identity(base, self, old.frame(), new.frame())?;
        if old_sources.corridor_source(base, old.corridor())?
            != new_sources.corridor_source(self, new.corridor())?
        {
            return Err(Failure::Unavailable);
        }
        let base_relations = base.traffic().relations();
        let target_relations = self.traffic().relations();
        same_identity(
            base,
            self,
            base_relations
                .corridor_reference_section(old.corridor())
                .ok_or(Failure::Binding)?,
            target_relations
                .corridor_reference_section(new.corridor())
                .ok_or(Failure::Binding)?,
        )?;
        let previous_rows = surfaces(base, old);
        let next_rows = surfaces(self, new);
        if previous_rows.is_empty()
            || previous_rows.len() != next_rows.len()
            || !previous_rows.iter().zip(next_rows).all(|(a, b)| {
                a.stations_mm == b.stations_mm
                    && a.jets == b.jets
                    && a.derivative_errors == b.derivative_errors
            })
        {
            return Err(Failure::Binding);
        }
        let base_pair = lane_pair(base, old.corridor(), old.window(), || Ok(()))?;
        let target_pair = lane_pair(self, new.corridor(), new.window(), || Ok(()))?;
        for (a, b) in base_pair.into_iter().zip(target_pair) {
            same_identity(base, self, a, b)?;
            same_identity(
                base,
                self,
                base_relations
                    .authoring_section(a)
                    .ok_or(Failure::Binding)?,
                target_relations
                    .authoring_section(b)
                    .ok_or(Failure::Binding)?,
            )?;
            let x = base
                .station_mapping()
                .lane_footprint(a)
                .ok_or(Failure::Unavailable)?;
            let y = self
                .station_mapping()
                .lane_footprint(b)
                .ok_or(Failure::Unavailable)?;
            if x.ends_mm != y.ends_mm
                || x.error_mm != y.error_mm
                || !x.supports_wheel
                || !y.supports_wheel
            {
                return Err(Failure::Binding);
            }
        }
        for (a, b) in old.window().edges.into_iter().zip(new.window().edges) {
            same_identity(
                base,
                self,
                project_window(base, old, a)?,
                project_window(self, new, b)?,
            )?;
        }
        let source = base_relations
            .lane_edge_authoring_lane(old.window().edges[0])
            .ok_or(Failure::Binding)?;
        let direction = if source == base_pair[0] { 1 } else { 2 };
        window_open(base, old, direction)?;
        window_open(self, new, direction)?;
        Ok(new)
    }
}
