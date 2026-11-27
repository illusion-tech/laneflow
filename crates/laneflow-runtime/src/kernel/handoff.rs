//! 已给定候选路线的一次横向交接试算。

use std::sync::Arc;

use laneflow_static_contract::{LaneEdgeOrdinal, NetworkRevisionId, VehicleProfileId};
use laneflow_static_network::{
    BoundaryOwner, SharedNetworkRevision, StationQuery,
    vehicle::{
        AxleDimensions, BodyEnvelope, Failure, GeometryWorkLimits, HandoffBinding,
        HandoffBodyReport, HandoffOutcome, HandoffQuery, HandoffReport, HandoffSourceEvidence,
        HandoffTravelReport, HandoffTravelTarget, HandoffWindow, SteeringLimits,
    },
};
use thiserror::Error;

use super::tables::{ManeuverOccurrence, access_cell_denied, compile_route, route_access_denied};
use crate::{RouteError, RouteRegisterInput, TrafficWorld};

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

/// 可在同一修订上多次试算的出现对，持有源修订并保留路线的原有出现顺序。
pub struct BoundRouteHandoff {
    revision: Arc<SharedNetworkRevision>,
    source_index: u32,
    boundary: HandoffBinding,
    legs: [RouteLeg; 2],
}

#[derive(Debug)]
struct RouteLeg {
    edges: Vec<LaneEdgeOrdinal>,
    maneuvers: Vec<ManeuverOccurrence>,
}

impl std::fmt::Debug for BoundRouteHandoff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoundRouteHandoff")
            .field("source_index", &self.source_index)
            .field("boundary", &self.boundary)
            .finish_non_exhaustive()
    }
}

impl BoundRouteHandoff {
    /// 该出现对的有向边界、所属走廊、框架和共同站位范围。
    #[must_use]
    pub fn boundary(&self) -> &HandoffBinding {
        &self.boundary
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
    /// 在命令路径预检路线容量与两段普通机动，将指定的相邻出现绑定到实际路层和允许窗口。
    /// 绑定保留两段路线的边和机动，可在同一修订反复试算。世界路线槽仍可用。
    ///
    /// # Errors
    ///
    /// 修订、出现下标、边界窗口、普通路线连通性或容量不满足时返回错误。
    ///
    /// # Panics
    ///
    /// 世界因执行 panic 失效后调用会 panic。
    pub fn bind_route_handoff(
        &self,
        route: &RouteRegisterInput,
        occurrence: RouteHandoffOccurrence,
    ) -> Result<BoundRouteHandoff, RouteHandoffTrialError> {
        self.execution.assert_usable();
        let revision = &self.state.binding.revision;
        if occurrence.revision != revision.network_revision() {
            return Err(RouteHandoffTrialError::RevisionMismatch);
        }
        let edges = route.edges();
        self.state.preflight_route_registration(edges.len())?;
        let target = (occurrence.source_index as usize)
            .checked_add(1)
            .filter(|&i| i < edges.len())
            .ok_or(RouteHandoffTrialError::OccurrenceOutOfRange)?;
        let crossing = [edges[target - 1], edges[target]];
        let boundary = revision
            .bind_handoff_window(
                occurrence.revision,
                HandoffWindow {
                    edges: crossing,
                    owner: occurrence.owner,
                    left_index: occurrence.left_index,
                    stations_nm: occurrence.stations_nm,
                },
            )
            .map_err(RouteHandoffTrialError::Boundary)?;
        self.compile_handoff(edges, occurrence.source_index, boundary)
    }

    /// 在目标世界重绑另一修订的候选路线。保持完整出现顺序并重新编译普通机动。
    /// 两端证据须能证明原有效窗口的物理对应关系。成功和失败均不修改两个世界。
    ///
    /// # Errors
    ///
    /// 来源、窗口、路线边或普通机动无法重绑，或目标容量不足时返回错误。
    ///
    /// # Panics
    ///
    /// 世界因执行 panic 失效后调用会 panic。
    pub fn rebind_route_handoff(
        &self,
        old: &BoundRouteHandoff,
        old_sources: &HandoffSourceEvidence<'_>,
        new_sources: &HandoffSourceEvidence<'_>,
    ) -> Result<BoundRouteHandoff, RouteHandoffTrialError> {
        self.execution.assert_usable();
        let target_revision = &self.state.binding.revision;
        let [source, target] = &old.legs;
        let count = source.edges.len() + target.edges.len();
        self.state.preflight_route_registration(count)?;
        let boundary = target_revision
            .rebind_handoff_window(&old.revision, old.boundary, old_sources, new_sources)
            .map_err(RouteHandoffTrialError::Boundary)?;
        let mut edges = Vec::new();
        edges
            .try_reserve_exact(count)
            .map_err(|_| RouteError::AllocationFailed)?;
        for edge in source.edges.iter().chain(&target.edges) {
            let stable = old
                .revision
                .identity()
                .stable_id(*edge)
                .ok_or(RouteError::UnknownEdge)?;
            edges.push(
                target_revision
                    .identity()
                    .ordinal(stable)
                    .ok_or(RouteError::UnknownEdge)?,
            );
        }
        self.compile_handoff(&edges, old.source_index, boundary)
    }

    fn compile_handoff(
        &self,
        edges: &[LaneEdgeOrdinal],
        source_index: u32,
        boundary: HandoffBinding,
    ) -> Result<BoundRouteHandoff, RouteHandoffTrialError> {
        let revision = &self.state.binding.revision;
        // 初次绑定已核对该下标有后继；重绑保留每个出现的位置。
        let target = source_index as usize + 1;
        let [source_edge, target_edge] = boundary.window().edges;
        if [edges[target - 1], edges[target]] != [source_edge, target_edge] {
            return Err(RouteHandoffTrialError::Boundary(Failure::Binding));
        }
        if revision
            .traffic()
            .maneuvers()
            .transition_candidates(source_edge)
            .is_some_and(|candidates| candidates.iter().any(|hop| hop.successor() == target_edge))
        {
            return Err(RouteError::ManeuverMismatch.into());
        }
        let mut live_conflicts = self.state.committed.live_route_conflict_occurrence_count;
        let mut compile_leg = |edges: &[LaneEdgeOrdinal]| -> Result<RouteLeg, RouteError> {
            let compiled = compile_route(
                revision,
                edges,
                live_conflicts,
                self.state
                    .binding
                    .config
                    .route_conflict_occurrence_capacity(),
            )?;
            live_conflicts += compiled.conflicts.len() as u64;
            Ok(RouteLeg {
                edges: compiled.edges,
                maneuvers: compiled.maneuvers,
            })
        };
        let (source, target) = edges.split_at(target);
        Ok(BoundRouteHandoff {
            revision: Arc::clone(revision),
            source_index,
            boundary,
            legs: [compile_leg(source)?, compile_leg(target)?],
        })
    }

    /// 在当前修订只读试算已绑定的路线出现对。复用绑定时编译的普通机动，
    /// 核对全部路线边与机动的静态通行规则，再按实际边和窗口求车体几何。
    /// 对已定位的交接，还须证明两侧车道在整车投影窗口内覆盖的每条边均允许该车型。
    /// 若投影窗口包含该车型禁行的边，即使车身实际未触及该边，也会返回
    /// `SideAccessUnproven`。调用方宜给出能覆盖整车的紧凑窗口。
    /// 此入口供命令路径使用。按车型检查两段路线的成本在几何预算之外。
    /// 动态跟车、信号与同拍占用仍须在步进中裁决。调用不登记路线或改变世界状态。
    ///
    /// # Errors
    ///
    /// 修订不匹配、车型不存在、路线准入被拒或两侧车道准入未证时返回相应错误。
    /// 几何拒绝、越窗或未证由报告区分。
    ///
    /// # Panics
    ///
    /// 世界因执行 panic 失效后调用会 panic。
    pub fn preview_route_handoff(
        &self,
        binding: &BoundRouteHandoff,
        trial: &RouteHandoffTrial,
    ) -> Result<RouteHandoffPreview<'_>, RouteHandoffTrialError> {
        self.execution.assert_usable();
        let revision = &self.state.binding.revision;
        if binding.boundary.revision() != revision.network_revision() {
            return Err(RouteHandoffTrialError::RevisionMismatch);
        }
        let traffic = revision.traffic();
        let profile_ordinal = revision
            .identity()
            .ordinal(trial.profile)
            .ok_or(RouteHandoffTrialError::UnknownProfile)?;
        let profile = traffic
            .relations()
            .vehicle_profile(profile_ordinal)
            .ok_or(RouteHandoffTrialError::UnknownProfile)?;
        for leg in &binding.legs {
            if route_access_denied(
                traffic,
                profile.class(),
                &leg.edges,
                0,
                leg.maneuvers
                    .iter()
                    .map(|m| (m.path, m.exit_route_edge_index)),
            ) {
                return Err(RouteHandoffTrialError::AccessDenied);
            }
        }
        let window = binding.boundary.window();
        let g = profile.geometry();
        let query = HandoffQuery {
            revision: binding.boundary.revision(),
            corridor: binding.boundary.corridor(),
            frame: binding.boundary.frame(),
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
            for edge in window.edges {
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
            }
        }
        Ok(RouteHandoffPreview {
            source_index: binding.source_index,
            trajectory: matches!(report.outcome, HandoffOutcome::Located { .. }).then_some(
                LocatedTrajectory {
                    revision,
                    query,
                    window,
                    body,
                },
            ),
            report,
            side_access_work,
        })
    }
}
