use super::RouteHandoffPreview;
use laneflow_static_contract::LaneEdgeOrdinal;
use laneflow_static_network::{
    StationCandidates,
    vehicle::{
        Failure, GeometryWorkLimits, HandoffFootprint, HandoffLaneSide, HandoffStationRange,
        HandoffTravelTarget, PoseBounds,
    },
};
use thiserror::Error;

/// 一拍已裁决的后轴行程所到达的姿态，以及前杠在原路线上的出现下标。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteHandoffSample {
    /// 查询指定的累计三维行程，单位毫米，两种行驶方向均递增。
    pub rear_travel_mm: u64,
    /// 包含后轴全部可能走廊站位的整数毫米闭区间，两端向外取整。
    pub rear_stations_mm: [u64; 2],
    /// 前保险杠中点所在的路线出现。边内进度闭区间见 `footprint.front`。
    pub front_occurrence: u32,
    /// 该姿态下前杠及两侧车体的证据。它只描述本拍末的位置。
    pub footprint: HandoffFootprint,
}

/// 内核在后续扫过中复用这里的原始站位界，毫米展示范围仅在公开查询时生成。
pub(crate) struct LocatedSample {
    pub(super) rear: HandoffStationRange,
    pub(super) front_occurrence: u32,
    pub(crate) footprint: HandoffFootprint,
}

/// 拟议的单拍行程无法完整定位。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum RouteHandoffSampleError {
    /// 累计行程溢出或超出可表示范围。
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
    pub(super) fn indices(self) -> std::ops::Range<usize> {
        self.first as usize..self.first as usize + self.edges.len()
    }

    pub(super) fn in_route(
        edges: &'a [LaneEdgeOrdinal],
        indices: std::ops::Range<usize>,
    ) -> Option<Self> {
        Some(Self {
            first: u32::try_from(indices.start).ok()?,
            edges: edges.get(indices)?,
        })
    }
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

    pub(super) fn resolve(self, edge: LaneEdgeOrdinal) -> Result<u32, RouteHandoffSampleError> {
        self.edges
            .iter()
            .position(|&candidate| candidate == edge)
            .and_then(|offset| self.first.checked_add(u32::try_from(offset).ok()?))
            .ok_or(RouteHandoffSampleError::FrontOutsideRoute { edge })
    }
}

impl RouteHandoffPreview<'_> {
    /// 以已裁决的本拍位移（毫米）推进一次只读取样。零位移可复核同一姿态。
    /// 两项行程相加后按给定的正整数毫米容差反查，并在整个结果闭区间核验前杠和车身。
    /// `limits.tracks` 限定行程反查，`limits.sweep` 限定位置求证。
    /// 超出候选终点时保留错误，调用方须单独裁决候选外的剩余行程。
    /// 未获原路线几何准入时返回 `None`。本方法不提交运动或处理预约。
    #[must_use]
    pub fn sample_after_travel(
        &self,
        rear_travel_mm: u64,
        step_travel_mm: u64,
        tolerance_mm: std::num::NonZeroU32,
        limits: GeometryWorkLimits,
    ) -> Option<RouteHandoffSampleReport> {
        let path = self.trajectory.as_ref()?;
        let Some(distance_mm) = rear_travel_mm
            .checked_add(step_travel_mm)
            .filter(|&mm| HandoffTravelTarget::DistanceMm(mm).is_representable())
        else {
            return Some(RouteHandoffSampleReport {
                sample: Err(RouteHandoffSampleError::DistanceOverflow),
                travel_work: 0,
                footprint_work: 0,
            });
        };
        let (result, travel_work, footprint_work) =
            path.sample_geometry(distance_mm, 0, u64::from(tolerance_mm.get()), limits, false);
        Some(RouteHandoffSampleReport {
            sample: result.map(|(sample, _)| RouteHandoffSample {
                rear_travel_mm: distance_mm,
                rear_stations_mm: sample.rear.millimetres(),
                front_occurrence: sample.front_occurrence,
                footprint: sample.footprint,
            }),
            travel_work,
            footprint_work,
        })
    }
}

type GeometrySample = Result<(LocatedSample, PoseBounds), RouteHandoffSampleError>;

impl super::LocatedTrajectory<'_> {
    pub(super) fn sample_geometry(
        &self,
        distance_mm: u64,
        carry: u16,
        tolerance_mm: u64,
        limits: GeometryWorkLimits,
        reserved: bool,
    ) -> (GeometrySample, u32, u32) {
        let path = self;
        let mut travel_work = 0;
        let mut footprint_work = 0;
        let mut run = || {
            let travel = path.revision.handoff_travel_after_motion(
                &path.query,
                path.window,
                distance_mm,
                carry,
                tolerance_mm,
                limits.tracks,
            );
            travel_work = travel.work;
            let rear = travel.bounds.map_err(RouteHandoffSampleError::Travel)?.rear;
            let evaluate = if reserved {
                laneflow_static_network::SharedNetworkRevision::reserved_handoff_footprint_and_pose
            } else {
                laneflow_static_network::SharedNetworkRevision::handoff_footprint_and_pose
            };
            let (geometry, used) = evaluate(
                path.revision,
                &path.query,
                path.window,
                path.body,
                rear,
                limits.sweep,
            );
            footprint_work = used;
            let (footprint, pose) = geometry.map_err(RouteHandoffSampleError::Footprint)?;
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
            Ok((
                LocatedSample {
                    rear,
                    front_occurrence,
                    footprint,
                },
                pose,
            ))
        };
        (run(), travel_work, footprint_work)
    }
}
