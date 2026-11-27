//! 候选路线的横向交接窗口与逐出现试算。

mod binding;

use std::sync::Arc;

use laneflow_static_contract::{LaneEdgeOrdinal, NetworkRevisionId, VehicleProfileId};
use laneflow_static_network::{
    BoundaryOwner, SharedNetworkRevision, StationQuery,
    vehicle::{
        AxleDimensions, BodyEnvelope, Failure, GeometryWorkLimits, HandoffBinding,
        HandoffBodyReport, HandoffOutcome, HandoffQuery, HandoffReport, HandoffTravelReport,
        HandoffTravelTarget, HandoffWindow, SteeringLimits,
    },
};
use thiserror::Error;

mod sample;
mod traffic;
use sample::CursorRun;
pub use sample::{RouteHandoffSample, RouteHandoffSampleError, RouteHandoffSampleReport};
pub use traffic::{RouteHandoffTraffic, RouteHandoffTrafficError};

mod clearance_braking;
mod clearance_curve;
mod clearance_plan;
pub use clearance_braking::{RouteHandoffBrakingError, RouteHandoffBrakingStep};
pub use clearance_curve::{
    ClearanceSpeedCurve, ClearanceSpeedCurveError, ClearanceSpeedPoint, MAX_CLEARANCE_SPEED_POINTS,
};
pub use clearance_plan::{
    ClearanceSpeedWindow, HandoffDrivingConfig, RouteHandoffSpeedPlan, RouteHandoffSpeedPlanError,
};

use super::tables::{ManeuverOccurrence, access_cell_denied, route_access_denied};
use crate::{RouteError, TrafficWorld};

/// 候选路线中某一次横向交接的来源声明。边由下标从完整路线读取。
#[derive(Clone, Copy, Debug)]
pub struct RouteHandoffOccurrence {
    /// 完整路线和结构边界所属的修订。
    pub revision: NetworkRevisionId,
    /// 源出现下标，紧邻的下一项为目标出现。
    pub source_index: u32,
    /// 两条车道之间的结构边界所有者。
    pub owner: BoundaryOwner,
    /// 该边界的左成员序号。
    pub left_index: u32,
    /// 允许前保险杠交接的闭区间，走廊局部纳米站位。
    pub stations_nm: [u64; 2],
}

/// 已绑定一处或多处交接窗口的候选路线。边与普通机动均使用完整路线的出现下标。
pub struct BoundRouteHandoff {
    revision: Arc<SharedNetworkRevision>,
    edges: Vec<LaneEdgeOrdinal>,
    maneuvers: Vec<ManeuverOccurrence>,
    windows: Vec<(u32, HandoffBinding)>,
}

impl std::fmt::Debug for BoundRouteHandoff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoundRouteHandoff")
            .field("windows", &self.windows)
            .finish_non_exhaustive()
    }
}

impl BoundRouteHandoff {
    /// 按源出现下标严格递增的有向边界及交接窗口。
    #[must_use]
    pub fn windows(&self) -> &[(u32, HandoffBinding)] {
        &self.windows
    }

    /// 取得指定出现对的有向边界、走廊、框架和共同站位范围。
    #[must_use]
    pub fn boundary_at(&self, source_index: u32) -> Option<&HandoffBinding> {
        let index = self.window_index(source_index)?;
        Some(&self.windows[index].1)
    }

    fn window_index(&self, source_index: u32) -> Option<usize> {
        self.windows
            .binary_search_by_key(&source_index, |w| w.0)
            .ok()
    }

    fn occurrence_legs(
        &self,
        source_index: u32,
    ) -> Option<(&[LaneEdgeOrdinal], &[LaneEdgeOrdinal])> {
        let index = self.window_index(source_index)?;
        let start = index
            .checked_sub(1)
            .map_or(0, |i| self.windows[i].0 as usize + 1);
        let split = source_index as usize + 1;
        let end = self
            .windows
            .get(index + 1)
            .map_or(self.edges.len(), |w| w.0 as usize + 1);
        Some((&self.edges[start..split], &self.edges[split..end]))
    }
}

/// 在已绑定出现对上尝试的一条轨迹，按稳定身份从绑定所属修订读取车型。
#[derive(Clone, Copy, Debug)]
pub struct RouteHandoffTrial {
    /// 用于求证全部静态几何与通行规则的车型。
    pub profile: VehicleProfileId,
    /// 允许求高与车身投影的走廊站位闭区间，单位纳米。两侧通行准入覆盖整个区间。
    pub station_window_nm: [u64; 2],
    /// 沿行驶方向排列的后轴起止站位，单位纳米。
    pub rear_stations_nm: [u64; 2],
    /// 此次候选的后轴三维车速，单位为米每秒。
    pub speed_m_s: f64,
    /// 一次完整几何查询的工作上限。
    pub limits: GeometryWorkLimits,
}

/// 候选路线上的静态交接试算结果，不含可执行的路线或预约句柄。
pub struct RouteHandoffPreview<'a> {
    /// 本次试算指定的源出现下标。
    pub source_index: u32,
    /// 实际求得的车身几何、前保险杠交接及工作计数。
    pub report: HandoffReport<'a>,
    /// 两侧准入检查的站位映射调用次数，消耗同一候选剩余的扫过预算。
    pub side_access_work: u32,
    trajectory: Option<LocatedTrajectory<'a>>,
}

struct LocatedTrajectory<'a> {
    revision: &'a SharedNetworkRevision,
    cursor_runs: [CursorRun<'a>; 2],
    query: HandoffQuery,
    window: HandoffWindow,
    body: BodyEnvelope,
}

impl std::fmt::Debug for RouteHandoffPreview<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RouteHandoffPreview")
            .field("source_index", &self.source_index)
            .field("report", &self.report)
            .field("side_access_work", &self.side_access_work)
            .finish_non_exhaustive()
    }
}

impl RouteHandoffPreview<'_> {
    /// 对已定位且两侧准入通过的同一条候选，查询后轴三维行程或从行程反算站位。
    /// 未定位的候选返回 `None`。容差以纳米计，此次求界独立计量工作。
    #[must_use]
    pub fn travel(
        &self,
        target: HandoffTravelTarget,
        tolerance_nm: u64,
        work_limit: u32,
    ) -> Option<HandoffTravelReport> {
        let path = self.trajectory.as_ref()?;
        Some(path.revision.handoff_travel(
            &path.query,
            path.window,
            target,
            tolerance_nm,
            work_limit,
        ))
    }

    /// 在同一候选的后轴站位闭区间内，证明整车相对结构边界处于哪一侧。
    /// 区间按站位递增给出。跨侧状态改变或预算耗尽时保留未证结果。
    /// 此查询只判断车体；下游空间与预约的释放另由当拍交通裁决。
    #[must_use]
    pub fn body_side(
        &self,
        rear_stations_nm: [u64; 2],
        work_limit: u32,
    ) -> Option<HandoffBodyReport> {
        let path = self.trajectory.as_ref()?;
        Some(path.revision.handoff_body_side(
            &path.query,
            path.window,
            path.body,
            rear_stations_nm,
            work_limit,
        ))
    }
}

/// 候选无法绑定到路线或证明车型静态准入。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum RouteHandoffTrialError {
    /// 路线或可用容量不满足同世界的路线注册约束。
    #[error(transparent)]
    Route(#[from] RouteError),
    /// 候选引用的序号属于别的路网修订。
    #[error("交接候选的路网修订不匹配")]
    RevisionMismatch,
    /// 出现对的结构边界、方向或共同站位范围无法绑定。
    #[error("交接出现对的边界或窗口无法绑定：{0:?}")]
    Boundary(Failure),
    /// 源出现缺少紧邻的下一条目标出现。
    #[error("交接出现对超出路线范围")]
    OccurrenceOutOfRange,
    /// 窗口清单为空或源出现下标未严格递增。
    #[error("交接窗口清单须非空且源出现下标严格递增")]
    InvalidWindowOrder,
    /// 多窗口路线的试算须明确指定交接下标。
    #[error("试算多窗口路线须指定源出现下标")]
    OccurrenceRequired,
    /// 指定下标处没有绑定交接窗口。
    #[error("指定的源出现下标处没有交接窗口")]
    UnboundOccurrence,
    /// 当前修订没有这项车型身份。
    #[error("交接试算的车辆配置不存在")]
    UnknownProfile,
    /// 路线的普通边或完整机动拒绝该车型的通行类别。
    #[error("交接候选路线不允许该车型通行")]
    AccessDenied,
    /// 该侧整车投影窗口的映射未能完整求出，或涉及不能证明允许该车型的边。
    #[error("交接侧 {route_edge:?} 无法证明整车均获准通行")]
    SideAccessUnproven {
        /// 对应侧在交接出现对中的路线边。
        route_edge: LaneEdgeOrdinal,
    },
}

impl TrafficWorld {
    /// 试算只有一处交接的候选路线。多窗口路线应调用 [`Self::preview_route_handoff_at`]。
    ///
    /// # Errors
    ///
    /// 多个窗口返回 [`RouteHandoffTrialError::OccurrenceRequired`]。
    /// 其余错误同 [`Self::preview_route_handoff_at`]。
    ///
    /// # Panics
    ///
    /// 世界因执行 panic 失效后调用会 panic。
    pub fn preview_route_handoff<'a>(
        &'a self,
        binding: &'a BoundRouteHandoff,
        trial: &RouteHandoffTrial,
    ) -> Result<RouteHandoffPreview<'a>, RouteHandoffTrialError> {
        self.execution.assert_usable();
        let [(source_index, _)] = binding.windows.as_slice() else {
            return Err(RouteHandoffTrialError::OccurrenceRequired);
        };
        self.preview_route_handoff_at(binding, *source_index, trial)
    }

    /// 在当前修订只读试算指定的路线出现对。复用绑定时编译的普通机动，
    /// 核对全部路线边与机动的静态通行规则，再按实际边和窗口求车体几何。
    /// 对已定位的交接，还须证明两侧车道在整车投影窗口内覆盖的每条边均允许该车型。
    /// 若投影窗口包含该车型禁行的边，即使车身实际未触及该边，也会返回
    /// `SideAccessUnproven`。调用方宜给出能覆盖整车的紧凑窗口。
    /// 此入口供命令路径使用。按车型检查整条路线的成本在几何预算之外。
    /// 动态跟车、信号与同拍占用仍须在步进中裁决。调用不登记路线或改变世界状态。
    ///
    /// # Errors
    ///
    /// 修订不匹配、出现下标未绑定、车型不存在、路线准入被拒或两侧车道准入未证时返回相应错误。
    /// 几何拒绝、越窗或未证由报告区分。
    ///
    /// # Panics
    ///
    /// 世界因执行 panic 失效后调用会 panic。
    pub fn preview_route_handoff_at<'a>(
        &'a self,
        binding: &'a BoundRouteHandoff,
        source_index: u32,
        trial: &RouteHandoffTrial,
    ) -> Result<RouteHandoffPreview<'a>, RouteHandoffTrialError> {
        self.execution.assert_usable();
        let revision = &self.state.binding.revision;
        if binding.revision.network_revision() != revision.network_revision() {
            return Err(RouteHandoffTrialError::RevisionMismatch);
        }
        let boundary = binding
            .boundary_at(source_index)
            .ok_or(RouteHandoffTrialError::UnboundOccurrence)?;
        let traffic = revision.traffic();
        let profile_ordinal = revision
            .identity()
            .ordinal(trial.profile)
            .ok_or(RouteHandoffTrialError::UnknownProfile)?;
        let profile = traffic
            .relations()
            .vehicle_profile(profile_ordinal)
            .ok_or(RouteHandoffTrialError::UnknownProfile)?;
        if route_access_denied(
            traffic,
            profile.class(),
            &binding.edges,
            0,
            binding
                .maneuvers
                .iter()
                .map(|m| (m.path, m.exit_route_edge_index)),
        ) {
            return Err(RouteHandoffTrialError::AccessDenied);
        }
        let window = boundary.window();
        let g = profile.geometry();
        let query = HandoffQuery {
            revision: boundary.revision(),
            corridor: boundary.corridor(),
            frame: boundary.frame(),
            station_window_nm: trial.station_window_nm,
            rear_stations_nm: trial.rear_stations_nm,
            axles: AxleDimensions {
                wheelbase_m: f64::from(g.wheelbase_mm) / 1_000.,
                half_track_m: f64::from(g.track_width_mm) / 2_000.,
                reference_height_m: f64::from(g.reference_height_mm) / 1_000.,
            },
        };
        let body = BodyEnvelope {
            rear_overhang_m: f64::from(g.rear_overhang_mm) / 1_000.,
            front_from_rear_m: f64::from(profile.length_mm() - g.rear_overhang_mm) / 1_000.,
            half_width_m: f64::from(g.width_mm) / 2_000.,
            heights_m: [g.bottom_height_mm, g.top_height_mm].map(|mm| f64::from(mm) / 1_000.),
        };
        let steering = SteeringLimits {
            max_angle_rad: f64::from(g.max_steering_angle_rad),
            max_rate_rad_s: f64::from(g.max_steering_rate_rad_s),
            speed_m_s: trial.speed_m_s,
        };
        let report = revision.vehicle_handoff(&query, steering, body, trial.limits, window);
        let mut side_access_work = 0;
        let trajectory = if let HandoffOutcome::Located { .. } = report.outcome {
            let (source, target) = binding
                .occurrence_legs(source_index)
                .ok_or(RouteHandoffTrialError::UnboundOccurrence)?;
            let mut cursor_runs = [CursorRun::EMPTY; 2];
            let sweep_used =
                report.work + report.geometry.map_or(0, |geometry| geometry.sweep.work);
            let sweep_limit = trial.limits.sweep.min(GeometryWorkLimits::STANDARD.sweep);
            for (side, edge) in window.edges.into_iter().enumerate() {
                let unproven = RouteHandoffTrialError::SideAccessUnproven { route_edge: edge };
                let lane = traffic
                    .relations()
                    .lane_edge_authoring_lane(edge)
                    .ok_or(unproven)?;
                if sweep_used + side_access_work >= sweep_limit {
                    return Err(unproven);
                }
                side_access_work += 1;
                // 扫过检查已将整车约束在此窗口内。它可包括未接触的边，窗口内的
                // 禁止只说明无法证明该侧准入，不能据此断言车辆实际驶入了禁区。
                let StationQuery::Complete(hits) = revision.station_mapping().in_station_range(
                    query.corridor,
                    trial.station_window_nm,
                    lane,
                ) else {
                    return Err(unproven);
                };
                if hits.addresses().any(|address| {
                    access_cell_denied(
                        traffic
                            .relations()
                            .edge_access(address.edge, profile.class()),
                    )
                }) {
                    return Err(unproven);
                }
                cursor_runs[side] = if side == 0 {
                    CursorRun::source(source, source_index, &hits)
                } else {
                    CursorRun::target(target, source_index, &hits)
                }
                .ok_or(unproven)?;
            }
            Some(LocatedTrajectory {
                revision,
                cursor_runs,
                query,
                window,
                body,
            })
        } else {
            None
        };
        Ok(RouteHandoffPreview {
            source_index,
            trajectory,
            report,
            side_access_work,
        })
    }
}
