use super::RouteHandoffPreview;
use laneflow_static_contract::LaneEdgeOrdinal;
use laneflow_static_network::{
    StationCandidates,
    vehicle::{
        Failure, GeometryWorkLimits, HandoffFootprint, HandoffLaneSide, HandoffTravelTarget,
    },
};
use thiserror::Error;

/// 一拍已裁决的后轴行程所到达的姿态，以及前杠在原路线上的出现下标。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteHandoffSample {
    /// 自候选起点累计的真实三维行程，单位微米，两种行驶方向均递增。
    pub rear_travel_um: u64,
    /// 行程反查给出的后轴走廊站位闭区间，单位纳米。
    pub rear_stations_nm: [u64; 2],
    /// 前保险杠中点所在的路线出现。边内进度闭区间见 `footprint.front`。
    pub front_occurrence: u32,
    /// 该姿态下前杠及两侧车体的证据。它只描述本拍末的位置。
    pub footprint: HandoffFootprint,
}

/// 拟议的单拍行程无法完整定位。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum RouteHandoffSampleError {
    /// 累计行程溢出或超出几何逆查可表示的纳米域。
    #[error("后轴累计行程超出可查询范围")]
    DistanceOverflow,
    /// 后轴三维行程逆查未证。
    #[error("后轴行程反查失败：{0:?}")]
    Travel(Failure),
    /// 无法在整个后轴站位域证明前保险杠与两侧车身的位置。
    #[error("横移车体位置未证：{0:?}")]
    Footprint(Failure),
    /// 前杠的可能位置跨越切点，无法唯一选取路线出现。
    #[error("前杠物理边不唯一")]
    AmbiguousFront,
    /// 前杠物理边不在本次出现对所衔接的连续车道链内。
    #[error("前杠物理边 {edge:?} 超出本次交接的路线")]
    FrontOutsideRoute {
        /// 几何查询定位的物理边。
        edge: LaneEdgeOrdinal,
    },
}

/// 一次按行程取样的完整结果和分别扣除的工作量。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteHandoffSampleReport {
    /// 任一阶段未证时保留原因。
    pub sample: Result<RouteHandoffSample, RouteHandoffSampleError>,
    /// 后轴三维行程逆查的工作数。
    pub travel_work: u32,
    /// 前杠与两侧车身定位的工作数。
    pub footprint_work: u32,
}

/// 从所选交接出现向行驶前方或后方，与路网物理车道链逐项吻合的子序列。
/// 直接借用路线边，其长度受已准入的边链映射候选上限约束。
#[derive(Clone, Copy)]
pub(super) struct CursorRun<'a> {
    first: u32,
    edges: &'a [LaneEdgeOrdinal],
}

impl<'a> CursorRun<'a> {
    pub(super) const EMPTY: Self = Self {
        first: 0,
        edges: &[],
    };

    pub(super) fn source(
        leg: &'a [LaneEdgeOrdinal],
        source_index: u32,
        hits: &StationCandidates,
    ) -> Option<Self> {
        let anchor = *leg.last()?;
        let through = hits.addresses().position(|h| h.edge == anchor)? + 1;
        let count = through.min(leg.len());
        let route = &leg[leg.len() - count..];
        let mut matched = 0_usize;
        for (hit, &edge) in hits.addresses().skip(through - count).zip(route) {
            // 只认紧邻本次源出现的后缀。更早的同名边可能属于另一次经过。
            if hit.edge == edge {
                matched += 1;
            } else {
                matched = 0;
            }
        }
        let predecessors = u32::try_from(matched.checked_sub(1)?).ok()?;
        let first = source_index.checked_sub(predecessors)?;
        Some(Self {
            first,
            edges: &leg[leg.len() - matched..],
        })
    }

    pub(super) fn target(
        leg: &'a [LaneEdgeOrdinal],
        source_index: u32,
        hits: &StationCandidates,
    ) -> Option<Self> {
        let anchor = *leg.first()?;
        let matched = hits
            .addresses()
            .skip_while(|h| h.edge != anchor)
            .zip(leg)
            .take_while(|(h, edge)| h.edge == **edge)
            .count();
        if matched == 0 {
            return None;
        }
        Some(Self {
            first: source_index.checked_add(1)?,
            edges: &leg[..matched],
        })
    }

    fn resolve(self, edge: LaneEdgeOrdinal) -> Result<u32, RouteHandoffSampleError> {
        self.edges
            .iter()
            .position(|&candidate| candidate == edge)
            .and_then(|offset| self.first.checked_add(u32::try_from(offset).ok()?))
            .ok_or(RouteHandoffSampleError::FrontOutsideRoute { edge })
    }
}

impl RouteHandoffPreview<'_> {
    /// 以已裁决的本拍位移（微米）推进一次只读取样。零位移可复核同一姿态。
    /// 两项行程相加后仅作一次纳米逆查，并在整个结果闭区间核验前杠和车身。
    /// `limits.tracks` 限定行程反查，`limits.sweep` 限定位置求证。
    /// 超出候选终点时保留错误，调用方须单独裁决候选外的剩余行程。
    /// 未获原路线几何准入时返回 `None`。本方法不提交运动或处理预约。
    #[must_use]
    pub fn sample_after_travel(
        &self,
        rear_travel_um: u64,
        step_travel_um: u64,
        tolerance_nm: u64,
        limits: GeometryWorkLimits,
    ) -> Option<RouteHandoffSampleReport> {
        let path = self.trajectory.as_ref()?;
        let mut travel_work = 0;
        let mut footprint_work = 0;
        let mut run = || {
            let distance = rear_travel_um
                .checked_add(step_travel_um)
                .ok_or(RouteHandoffSampleError::DistanceOverflow)?;
            let distance_nm = distance
                .checked_mul(1_000)
                .ok_or(RouteHandoffSampleError::DistanceOverflow)?;
            let travel = path.revision.handoff_travel(
                &path.query,
                path.window,
                HandoffTravelTarget::DistanceNm(distance_nm),
                tolerance_nm,
                limits.tracks,
            );
            travel_work = travel.work;
            let rear_stations_nm = travel
                .bounds
                .map_err(RouteHandoffSampleError::Travel)?
                .rear_stations_nm;
            let report = path.revision.handoff_footprint(
                &path.query,
                path.window,
                path.body,
                rear_stations_nm,
                limits.sweep,
            );
            footprint_work = report.work;
            let footprint = report
                .footprint
                .map_err(RouteHandoffSampleError::Footprint)?;
            let edge = {
                let mut front = footprint.front.addresses();
                match (front.next(), front.next()) {
                    (Some(address), None) => address.edge,
                    _ => return Err(RouteHandoffSampleError::AmbiguousFront),
                }
            };
            let leg = match footprint.front_side {
                HandoffLaneSide::Source => path.cursor_runs[0],
                HandoffLaneSide::Target => path.cursor_runs[1],
            };
            let front_occurrence = leg.resolve(edge)?;
            Ok(RouteHandoffSample {
                rear_travel_um: distance,
                rear_stations_nm,
                front_occurrence,
                footprint,
            })
        };
        Some(RouteHandoffSampleReport {
            sample: run(),
            travel_work,
            footprint_work,
        })
    }
}
