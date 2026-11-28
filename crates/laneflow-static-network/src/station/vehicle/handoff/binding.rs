use super::{Failure, HandoffWindow, lane_pair};
use crate::{SharedNetworkRevision, SharedStationMapping};
use core::ops::Bound::{self, Excluded, Included};
use laneflow_static_contract::{
    AuthoringLaneOrdinal, CanonicalFrameOrdinal, LaneEdgeOrdinal, NetworkRevisionId,
    RoadCorridorOrdinal,
};

/// 已按指定修订核对相邻车道、行驶方向和共同站位域的交接边对。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HandoffBinding {
    revision: NetworkRevisionId,
    window: HandoffWindow,
    corridor: RoadCorridorOrdinal,
    frame: CanonicalFrameOrdinal,
    increasing: bool,
    common_stations_mm: [u64; 2],
    open_travel_end: bool,
}

impl HandoffBinding {
    /// 所有序号所属的路网修订。
    #[must_use]
    pub const fn revision(&self) -> NetworkRevisionId {
        self.revision
    }

    /// 声明的源、目标边，有向结构边界及交接窗口。
    #[must_use]
    pub const fn window(&self) -> HandoffWindow {
        self.window
    }

    /// 两车道和所选结构边界共同所属的走廊。
    #[must_use]
    pub const fn corridor(&self) -> RoadCorridorOrdinal {
        self.corridor
    }

    /// 已发布走廊路面采用的规范坐标框架。
    #[must_use]
    pub const fn frame(&self) -> CanonicalFrameOrdinal {
        self.frame
    }

    /// 两条车道都沿走廊站位递增方向行驶时返回 `true`。
    #[must_use]
    pub const fn increasing(&self) -> bool {
        self.increasing
    }

    /// 两边与声明窗口的交集，按毫米站位从小到大。切点归行驶下游边，
    /// 上游端显式为开端点；行驶方向反转时开端点相应落在低端。
    #[must_use]
    pub const fn common_stations_mm(&self) -> [Bound<u64>; 2] {
        let [lo, hi] = self.common_stations_mm;
        match (self.increasing, self.open_travel_end) {
            (true, true) => [Included(lo), Excluded(hi)],
            (false, true) => [Excluded(lo), Included(hi)],
            (_, false) => [Included(lo), Included(hi)],
        }
    }

    pub(super) fn common_stations_nm(&self) -> [u64; 2] {
        // 发布站位已验证可表示。两个端点之间仅行驶终端可能是开端点。
        let [lo, hi] = self.common_stations_mm.map(|mm| mm * super::NM_PER_MM);
        [
            lo + u64::from(self.open_travel_end && !self.increasing),
            hi - u64::from(self.open_travel_end && self.increasing),
        ]
    }
}

struct EdgeDomain {
    lane: AuthoringLaneOrdinal,
    corridor: RoadCorridorOrdinal,
    increasing: bool,
    stations_mm: [u64; 2],
    last: bool,
}

/// 返回该边按行驶方向实际拥有的毫米站位域。内切点属于下游边，链末端属于末边。
fn owned_stations(
    mapping: &SharedStationMapping,
    ordinal: LaneEdgeOrdinal,
) -> Result<EdgeDomain, Failure> {
    let edge = mapping
        .edges
        .get(ordinal.index())
        .copied()
        .flatten()
        .ok_or(Failure::Unavailable)?;
    let lane = mapping.lanes[edge.lane.index()].ok_or(Failure::Unavailable)?;
    let spans = edge.spans.slice(&mapping.segments);
    let start = spans.first().ok_or(Failure::Unavailable)?.map.stations_mm()[0];
    let end = spans.last().ok_or(Failure::Unavailable)?.map.stations_mm()[1];
    let chain = lane.spans.slice(&mapping.segments);
    let last = chain.last().ok_or(Failure::Unavailable)?.edge == ordinal;
    let stations = [start.min(end), start.max(end)];
    Ok(EdgeDomain {
        lane: edge.lane,
        corridor: lane.corridor,
        increasing: lane.increasing,
        stations_mm: stations,
        last,
    })
}

impl SharedNetworkRevision {
    /// 绑定显式交接边对。按声明修订查找所属走廊和路面坐标系，并按有向切点归属
    /// 检查两条边与窗口具有非空共同站位。此查询不依赖车型、后轴轨迹或表现几何。
    ///
    /// # Errors
    ///
    /// 窗口倒序或越出走廊时返回 [`Failure::InvalidInput`]，站位映射或路面缺失时
    /// 返回 [`Failure::Unavailable`]，修订、结构边界、方向或共同站位域不符时返回
    /// [`Failure::Binding`]。
    pub fn bind_handoff_window(
        &self,
        revision: NetworkRevisionId,
        window: HandoffWindow,
    ) -> Result<HandoffBinding, Failure> {
        if self.network_revision() != revision {
            return Err(Failure::Binding);
        }
        let mapping = self.station_mapping();
        let source = owned_stations(mapping, window.edges[0])?;
        let target = owned_stations(mapping, window.edges[1])?;
        let corridor = source.corridor;
        if source.lane == target.lane
            || corridor != target.corridor
            || source.increasing != target.increasing
        {
            return Err(Failure::Binding);
        }
        let [low, high] = window.stations_mm;
        let domain = mapping
            .corridor_domain(corridor)
            .ok_or(Failure::Unavailable)?;
        if low > high || high > domain.length_mm() {
            return Err(Failure::InvalidInput);
        }
        let members = lane_pair(self, corridor, window, || Ok(()))?;
        if members != [source.lane, target.lane] && members != [target.lane, source.lane] {
            return Err(Failure::Binding);
        }
        let low = low.max(source.stations_mm[0]).max(target.stations_mm[0]);
        let high = high.min(source.stations_mm[1]).min(target.stations_mm[1]);
        let common = [low, high];
        let travel_end = usize::from(source.increasing);
        let open_travel_end = [&source, &target]
            .iter()
            .any(|edge| !edge.last && edge.stations_mm[travel_end] == common[travel_end]);
        if low > high || (low == high && open_travel_end) {
            return Err(Failure::Binding);
        }
        let frame = mapping
            .surface_spans(corridor)
            .first()
            .ok_or(Failure::Unavailable)?
            .frame;
        Ok(HandoffBinding {
            revision,
            window,
            corridor,
            frame,
            increasing: source.increasing,
            common_stations_mm: common,
            open_travel_end,
        })
    }
}
