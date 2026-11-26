//! 已给定候选路线的一次横向交接试算。

use laneflow_static_contract::{
    CanonicalFrameOrdinal, LaneEdgeOrdinal, NetworkRevisionId, RoadCorridorOrdinal,
    VehicleProfileOrdinal,
};
use laneflow_static_network::{
    BoundaryOwner, StationQuery,
    vehicle::{
        AxleDimensions, BodyEnvelope, GeometryWorkLimits, HandoffOutcome, HandoffQuery,
        HandoffReport, HandoffWindow, SteeringLimits,
    },
};
use thiserror::Error;

use super::tables::{access_cell_denied, compile_route, route_access_denied};
use crate::{RouteError, RouteRegisterInput, TrafficWorld};

/// 在候选路线的一对相邻出现记录之间试算横移。其余转换沿用普通路线规则。
/// 车型的准入类别、总长、轴几何、整车包络与转向限值均取自当前修订的同一车型。
pub struct RouteHandoffTrial<'a> {
    /// 绑定到所选修订的完整边序列。
    pub route: &'a RouteRegisterInput,
    /// 源出现下标，紧邻的下一项为目标出现。
    pub source_index: u32,
    /// 两条车道之间的结构边界所有者。
    pub owner: BoundaryOwner,
    /// 该边界的左成员序号。
    pub left_index: u32,
    /// 允许前保险杠交接的闭区间，走廊局部纳米站位。
    pub stations_nm: [u64; 2],
    /// 用于求证全部静态几何与通行规则的车型。
    pub profile: VehicleProfileOrdinal,
    /// 路线边、车型及横断面所属修订。
    pub revision: NetworkRevisionId,
    /// 所属路层的走廊。
    pub corridor: RoadCorridorOrdinal,
    /// 该修订发布的坐标框架。
    pub frame: CanonicalFrameOrdinal,
    /// 允许求高与车身投影的走廊站位闭区间，单位纳米。
    pub station_window_nm: [u64; 2],
    /// 沿行驶方向排列的后轴起止站位，单位纳米。
    pub rear_stations_nm: [u64; 2],
    /// 此次候选的后轴三维车速，单位为米每秒。
    pub speed_m_s: f64,
    /// 一次完整几何查询的工作上限。
    pub limits: GeometryWorkLimits,
}

/// 候选路线上的静态交接试算结果，不含可执行的路线或预约句柄。
#[derive(Debug)]
pub struct RouteHandoffPreview<'a> {
    /// 本次试算指定的源出现下标。
    pub source_index: u32,
    /// 实际求得的车身几何、前保险杠交接及工作计数。
    pub report: HandoffReport<'a>,
    /// 两侧准入检查的站位映射调用次数，消耗同一候选剩余的扫过预算。
    pub side_access_work: u32,
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
    /// 源出现缺少紧邻的下一条目标出现。
    #[error("交接出现对超出路线范围")]
    OccurrenceOutOfRange,
    /// 车型序号未绑定到当前修订。
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
    /// 在当前修订只读试算一对路线出现的横向交接。分别编译断点两侧的普通路线，
    /// 核对全部路线边与机动的静态通行规则，再按实际边和窗口求车体几何。
    /// 对已定位的交接，还须证明两侧车道在整车投影窗口内覆盖的每条边均允许该车型。
    /// 成本包含两段路线的临时编译存储；此入口供命令路径使用。
    /// 动态跟车、信号与同拍占用仍须在步进中裁决。调用不登记路线或改变世界状态。
    ///
    /// # Errors
    ///
    /// 路线编译或容量预检失败、修订不匹配、选定出现没有下一项、车型不存在、
    /// 路线准入被拒或两侧车道准入未证时返回相应错误。
    /// 几何拒绝、越窗或未证由报告区分。
    ///
    /// # Panics
    ///
    /// 世界因执行 panic 失效后调用会 panic。
    pub fn preview_route_handoff(
        &self,
        trial: &RouteHandoffTrial<'_>,
    ) -> Result<RouteHandoffPreview<'_>, RouteHandoffTrialError> {
        self.execution.assert_usable();
        let revision = &self.state.binding.revision;
        if trial.revision != revision.network_revision() {
            return Err(RouteHandoffTrialError::RevisionMismatch);
        }
        let edges = trial.route.edges();
        self.state.preflight_route_registration(edges.len())?;
        let target = (trial.source_index as usize)
            .checked_add(1)
            .filter(|&i| i < edges.len())
            .ok_or(RouteHandoffTrialError::OccurrenceOutOfRange)?;
        let traffic = revision.traffic();
        let profile = traffic
            .relations()
            .vehicle_profile(trial.profile)
            .ok_or(RouteHandoffTrialError::UnknownProfile)?;
        let crossing = [edges[target - 1], edges[target]];
        if traffic
            .maneuvers()
            .transition_candidates(crossing[0])
            .is_some_and(|candidates| candidates.iter().any(|hop| hop.successor() == crossing[1]))
        {
            return Err(RouteError::ManeuverMismatch.into());
        }

        // 在源出现后断开，两段分别按实际注册的机动覆盖、首尾 Gate 及容量规则编译。
        // 后段须继承前段已消耗的冲突出现容量。
        let mut live_conflicts = self.state.committed.live_route_conflict_occurrence_count;
        for leg in [&edges[..target], &edges[target..]] {
            let compiled = compile_route(
                revision,
                leg,
                live_conflicts,
                self.state
                    .binding
                    .config
                    .route_conflict_occurrence_capacity(),
            )?;
            if route_access_denied(
                traffic,
                profile.class(),
                leg,
                0,
                compiled
                    .maneuvers
                    .iter()
                    .map(|m| (m.path, m.exit_route_edge_index)),
            ) {
                return Err(RouteHandoffTrialError::AccessDenied);
            }
            live_conflicts += compiled.conflicts.len() as u64;
        }
        let window = HandoffWindow {
            edges: crossing,
            owner: trial.owner,
            left_index: trial.left_index,
            stations_nm: trial.stations_nm,
        };
        let g = profile.geometry();
        let query = HandoffQuery {
            revision: trial.revision,
            corridor: trial.corridor,
            frame: trial.frame,
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
        if let HandoffOutcome::Located { .. } = report.outcome {
            let sweep_used =
                report.work + report.geometry.map_or(0, |geometry| geometry.sweep.work);
            let sweep_limit = trial.limits.sweep.min(GeometryWorkLimits::STANDARD.sweep);
            for edge in crossing {
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
                    trial.corridor,
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
            }
        }
        Ok(RouteHandoffPreview {
            source_index: trial.source_index,
            report,
            side_access_work,
        })
    }
}
