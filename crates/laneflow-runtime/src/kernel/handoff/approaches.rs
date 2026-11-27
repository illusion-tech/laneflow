use std::ops::RangeInclusive;

use laneflow_static_network::{
    StationCandidates,
    vehicle::{Failure, GeometryWorkLimits, HandoffOutcome, HandoffTravelTarget, TRACK_WORK_LIMIT},
};
use thiserror::Error;

use super::{
    BoundRouteHandoff, LocatedTrajectory, RouteHandoffEntrantError, RouteHandoffEntrants,
    RouteHandoffTrial, RouteHandoffTrialError, entrants::check_entrants,
};
use crate::TrafficWorld;

/// 外部候选的一段连续运动在当前提交态遇到的普通车辆制动约束。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteHandoffApproaches {
    /// 源、目标侧可能被整个运动过程扫及的边内闭区间。外包相交表示可能阻挡。
    pub swept_sides: [Option<StationCandidates>; 2],
    /// 每侧首个可能已占用或无法证明能在外包前停住的车辆。
    pub entrants: RouteHandoffEntrants,
    /// 两个行程端点逆查共用轨迹预算。零位移只查一次。
    pub travel_work: u32,
    /// 两侧连续扫过投影使用的逻辑工作数。
    pub sweep_work: u32,
}

/// 验证外部横移的连续扫过与接近车辆时无法取得完整结果。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum RouteHandoffApproachError {
    /// 路线、出现、车型或准入条件不满足。
    #[error(transparent)]
    Trial(#[from] RouteHandoffTrialError),
    /// 原轨迹尚未证明可用。
    #[error("候选没有可验证的交接轨迹：{0:?}")]
    Unlocated(HandoffOutcome),
    /// 后轴累计行程倒序，或微米值超过逆查的纳米域。
    #[error("候选的累计后轴行程区间无效")]
    InvalidTravelRange,
    /// 无法在预算和指定容差内定位两端。
    #[error("累计行程端点未证：{reason:?}（轨迹工作 {work}）")]
    Travel {
        /// 几何逆查失败原因。
        reason: Failure,
        /// 两端累计消耗的轨迹工作数。
        work: u32,
    },
    /// 无法得到连续的两侧投影。
    #[error("连续扫过未证：{reason:?}（扫过工作 {work}）")]
    Sweep {
        /// 扫过几何失败原因。
        reason: Failure,
        /// 已消耗的扫过工作数。
        work: u32,
    },
    /// 进入者搜索未完成。
    #[error(transparent)]
    Entrants(#[from] RouteHandoffEntrantError),
}

impl LocatedTrajectory<'_> {
    fn rear_stations_over_travel(
        &self,
        travel_um: RangeInclusive<u64>,
        tolerance_nm: u64,
        track_limit: u32,
    ) -> Result<([u64; 2], u32), RouteHandoffApproachError> {
        if travel_um.is_empty() {
            return Err(RouteHandoffApproachError::InvalidTravelRange);
        }
        let (start, end) = travel_um.into_inner();
        let end_nm = end
            .checked_mul(1_000)
            .ok_or(RouteHandoffApproachError::InvalidTravelRange)?;
        let limit = track_limit.min(TRACK_WORK_LIMIT);
        let mut used = 0;
        let mut locate = |distance_nm| {
            let report = self.revision.handoff_travel(
                &self.query,
                self.window,
                HandoffTravelTarget::DistanceNm(distance_nm),
                tolerance_nm,
                limit - used,
            );
            used += report.work;
            report
                .bounds
                .map(|v| v.rear_stations_nm)
                .map_err(|reason| RouteHandoffApproachError::Travel { reason, work: used })
        };
        let first = locate(start * 1_000)?;
        let last = if start == end { first } else { locate(end_nm)? };
        // 累计行程沿候选严格递增；绝对走廊站位沿同一方向单调，在反向车道数值递减。
        Ok(([first[0].min(last[0]), first[1].max(last[1])], used))
    }
}

impl TrafficWorld {
    /// 核对外部候选在一段后轴累计行程中连续扫过的两侧及当前普通车辆能否让行。
    /// `travel_um` 是从同一候选起点算起的有序闭区间，单位微米，反向行驶同样递增。
    /// 路线与整条候选的准入复用逐出现试算。随后两端逆查共用一次轨迹预算，连续扫过
    /// 单独使用一次扫过预算。进入者查询受 `entrant_work_limit` 约束，包含所有已提交
    /// 活动车辆，适用于尚未进入世界的候选。结果不含信号、下游清空或同拍其他预约的裁决。
    ///
    /// # Errors
    ///
    /// 绑定、轨迹、行程域、连续扫过或进入者枚举无法完整求证时返回相应错误。
    ///
    /// # Panics
    ///
    /// 世界因执行 panic 失效后调用会 panic。
    pub fn preview_route_handoff_approaches(
        &self,
        binding: &BoundRouteHandoff,
        source_index: u32,
        trial: &RouteHandoffTrial,
        travel_um: RangeInclusive<u64>,
        tolerance_nm: u64,
        entrant_work_limit: u32,
    ) -> Result<RouteHandoffApproaches, RouteHandoffApproachError> {
        let preview = self.preview_route_handoff_at(binding, source_index, trial)?;
        let path = preview
            .trajectory
            .as_ref()
            .ok_or(RouteHandoffApproachError::Unlocated(preview.report.outcome))?;
        let GeometryWorkLimits { tracks, sweep } = trial.limits;
        let (stations, travel_work) =
            path.rear_stations_over_travel(travel_um, tolerance_nm, tracks)?;
        let report = path.revision.handoff_swept_occupancy(
            &path.query,
            path.window,
            path.body,
            stations,
            sweep,
        );
        let swept_sides = report
            .sides
            .map_err(|reason| RouteHandoffApproachError::Sweep {
                reason,
                work: report.work,
            })?;
        let entrants = check_entrants(
            &self.state,
            swept_sides.each_ref().map(Option::as_ref),
            entrant_work_limit,
        )?;
        Ok(RouteHandoffApproaches {
            swept_sides,
            entrants,
            travel_work,
            sweep_work: report.work,
        })
    }
}
