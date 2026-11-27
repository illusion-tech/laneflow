use super::{Failure, HandoffWindow, lane_pair};
use crate::{SharedNetworkRevision, SharedStationMapping};
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
    common_stations_nm: [u64; 2],
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

    /// 两边实际拥有且在声明窗口内的可表示纳米站位闭区间，从小到大。
    /// 源于边切点的半开拓扑归属。排除的高端用切点前一纳米表示，排除的低端用切点后一纳米表示。
    #[must_use]
    pub const fn common_stations_nm(&self) -> [u64; 2] {
        self.common_stations_nm
    }
}

struct EdgeDomain {
    lane: AuthoringLaneOrdinal,
    corridor: RoadCorridorOrdinal,
    increasing: bool,
    stations_nm: [u64; 2],
}

/// 返回该边按行驶方向实际拥有的整数纳米站位。内切点属于下游边，链末端属于末边。
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
    let start = spans.first().ok_or(Failure::Unavailable)?.stations_nm()[0];
    let end = spans.last().ok_or(Failure::Unavailable)?.stations_nm()[1];
    let chain = lane.spans.slice(&mapping.segments);
    let last = chain.last().ok_or(Failure::Unavailable)?.edge == ordinal;
    let stations = if lane.increasing {
        [start, if last { end } else { end - 1 }]
    } else {
        [if last { end } else { end + 1 }, start]
    };
    Ok(EdgeDomain {
        lane: edge.lane,
        corridor: lane.corridor,
        increasing: lane.increasing,
        stations_nm: stations,
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
        let [low, high] = window.stations_nm()?;
        let domain = mapping
            .corridor_domain(corridor)
            .ok_or(Failure::Unavailable)?;
        if low > high || high > domain.length_nm() {
            return Err(Failure::InvalidInput);
        }
        let members = lane_pair(self, corridor, window, || Ok(()))?;
        if members != [source.lane, target.lane] && members != [target.lane, source.lane] {
            return Err(Failure::Binding);
        }
        let common = [
            low.max(source.stations_nm[0]).max(target.stations_nm[0]),
            high.min(source.stations_nm[1]).min(target.stations_nm[1]),
        ];
        if common[0] > common[1] {
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
            common_stations_nm: common,
        })
    }
}
