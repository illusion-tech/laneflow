use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::*;
use laneflow_static_contract::*;
use laneflow_static_network::*;
use laneflow_synth_generator::{Bundle, InitialState};

use crate::{Error, failure};

pub(crate) struct Sample {
    pub world: TrafficWorld,
    pub state: InitialState,
    pub handles: Vec<VehicleHandle>,
    pub routes: Vec<RouteHandle>,
    pub groups: Vec<Option<SignalGroupOrdinal>>,
    pub gates: Vec<Option<ManeuverGateOrdinal>>,
    /// L 族首个换道簇路线下标（`None` 为 W 族）；簇路线无信号门。
    pub lane_change_route: Option<usize>,
    /// L 族全部换道簇路线下标（多簇，#1233）；与 `lane_change_route` 同步。
    pub lane_change_routes: Vec<usize>,
    /// L 族换道预约生命周期史（#1218 面 B 身份投影）：
    /// `(tick, 换道者槽位, 事件种类, 资源身份)`，事件序保持。
    pub lane_change_journal: Vec<(u64, u16, u32, u32)>,
    /// L1 观测跨拍生命周期序（#1227）：按换道者槽位记录最近一次生命周期事件
    /// 种类（4 acquire / 5 enter / 6 clear / 7 release），跨观测调用持续。
    pub lane_change_last_kind: Vec<u32>,
    /// 逐路线位置前缀和（毫米，含起点 0 与终点）：L 族边长非均匀，位置编码
    /// 由真实边长累计（W 族恒为 `EDGE_MM` 网格，不使用本表）。
    pub route_offsets: Vec<Vec<u64>>,
    pub initial: Vec<VehicleState>,
    pub previous: Vec<VehicleState>,
    pub bindings: Vec<Option<ParkingBinding>>,
    pub crossed: Vec<bool>,
    pub transitions: u64,
    /// 减速诊断：最大观测减速度、超 profile 计数及其具名例外归因（#1487）。
    pub deceleration: crate::decel::Counters,
    /// 车型最大加速度（毫米/秒²）：由共享路网 profile 视图派生，运动学语义
    /// 检查按车型上界判定（W 族 1.4、L 族 1.5，#1218）。
    pub max_accel_mm_s2: u32,
    /// 车型紧急减速度上限（毫米/秒²）：同一共享 profile 视图派生，减速
    /// 诊断按安装 profile 判超（W 族 6、L 族 9，#1243 T5）——不按场景族
    /// 硬编码第二套阈值，合法紧急制动不误计 `above_profile`。
    pub emergency_decel_mm_s2: u64,
    pub cursor: u64,
    pub trace_bytes: Vec<u8>,
    /// 共享验证会话（W1/W2 正式 run 接线；`None` 为 L 族专项或测试直装，
    /// 观测在有会话时记录规则触发、违反与见证，无会话时行为不变）。
    pub session: Option<laneflow_traffic_verification::session::Session>,
    /// 冻结输入身份（检查点/恢复对账用；空串表示未接线）。
    pub input_identity: String,
}

/// 由共享路网 profile 视图派生运动学判据上界（#1243 T5）：车型 ordinal、
/// 最大加速度与紧急减速（毫米/秒²）。安装与恢复两侧共用同一事实源。
pub(crate) fn profile_limits(
    revision: &SharedNetworkRevision,
    state: &InitialState,
) -> Result<(VehicleProfileOrdinal, u32, u64), Error> {
    let profile = revision
        .identity()
        .ordinal(
            state
                .vehicle_profile_id
                .parse::<VehicleProfileId>()
                .map_err(|e| failure("profile id", e))?,
        )
        .ok_or_else(|| failure("profile", "missing"))?;
    let view = revision
        .traffic()
        .relations()
        .vehicle_profile(profile)
        .ok_or_else(|| failure("profile view", "missing"))?;
    let max_accel_mm_s2 = (f64::from(view.max_accel()) * 1_000.0).round().max(0.0) as u32;
    let emergency_decel_mm_s2 = (f64::from(view.emergency_decel()) * 1_000.0)
        .round()
        .max(0.0) as u64;
    Ok((profile, max_accel_mm_s2, emergency_decel_mm_s2))
}

/// W 参考域校验（lf-synth-execution-2 §1）：独立信号许可参考判定的适用
/// 条件一次性核对，安装与恢复两侧共用——W 路线为 entry→exit 两边且 entry
/// 边长 5 130 m、每条信号路线唯一 Gate（安装循环已钉住）、门政策在全部
/// 参与者类别下解析为 `ProtectedGroup` 且无独立禁止、受检 profile 的紧急
/// 减速度原始值恰为 6.0 m/s²（派生整数 6 000）。四舍五入结果等于 6000
/// 不等于原始值精确——两者同时核对。换道族无信号门，不适用本判定。
/// 生成器演进后条件不再成立时在此失败，参考判定不得静默套用旧假设。
pub(crate) fn validate_w_signal_reference_domain(
    revision: &SharedNetworkRevision,
    state: &InitialState,
    route_offsets: &[Vec<u64>],
    emergency_decel_mm_s2: u64,
) -> Result<(), Error> {
    if state.identity.lane_change().is_some() {
        return Ok(());
    }
    let identity = revision.identity();
    let relations = revision.traffic().relations();
    let profile = identity
        .ordinal(
            state
                .vehicle_profile_id
                .parse::<VehicleProfileId>()
                .map_err(|e| failure("profile id", e))?,
        )
        .ok_or_else(|| failure("profile", "missing"))?;
    let profile_view = relations
        .vehicle_profile(profile)
        .ok_or_else(|| failure("profile view", "missing"))?;
    crate::require(
        profile_view.emergency_decel() == 6.0 && emergency_decel_mm_s2 == 6_000,
        || "参考许可要求受检 profile 紧急减速度恰为 6.0 m/s²（6000）".into(),
    )?;
    let policy = identity
        .ordinal(
            state
                .policy_id
                .parse::<RightOfWayPolicySetId>()
                .map_err(|e| failure("policy id", e))?,
        )
        .ok_or_else(|| failure("policy", "missing"))?;
    let policy_view = revision
        .policy()
        .policy(policy)
        .ok_or_else(|| failure("policy view", "missing"))?;
    for (route_rank, route) in state.routes.iter().enumerate() {
        let Some(group_id) = &route.signal_group_id else {
            continue;
        };
        crate::require(
            route.edge_ids.len() == 2 && route_offsets[route_rank][1] == 5_130_000,
            || format!("W 信号路线须为 entry→exit 且 entry 边长 5 130 m：route={route_rank}"),
        )?;
        let group = identity
            .ordinal(
                group_id
                    .parse::<SignalGroupId>()
                    .map_err(|e| failure("group id", e))?,
            )
            .ok_or_else(|| failure("group", "missing"))?;
        let gate = relations
            .signal_group(group)
            .ok_or_else(|| failure("group relations", "missing"))?
            .gates()[0];
        let classes = policy_view.gate_classes(gate);
        crate::require(
            !classes.is_empty()
                && classes.iter().all(|class| {
                    class.interpretation() == GateInterpretation::ProtectedGroup
                        && class.prohibition() == GateProhibition::None
                }),
            || format!("W 信号门政策须为 ProtectedGroup 且无独立禁止：route={route_rank}"),
        )?;
    }
    Ok(())
}

/// W 车身分离快路径的域绑定（设计 §5.2）：受检一维走廊要求每条路线的
/// 两条边恰为 `EDGE_MM` 网格（`position` 的“边序 × 网格 + 进度”坐标
/// 忠实反映物理空间）、路线内两边不别名、不同路线不共享物理边（不同
/// `rank / 25` 走廊不共享需互检的物理车身空间）。任一前提不成立即拒绝
/// 快路径，不仅凭 `rank / 25` 推断空间隔离；安装与恢复均执行。
pub(crate) fn validate_w_corridor_binding(
    state: &InitialState,
    route_offsets: &[Vec<u64>],
) -> Result<(), Error> {
    if state.identity.lane_change().is_some() {
        return Ok(());
    }
    let mut seen_edges = std::collections::HashSet::new();
    for (route_rank, route) in state.routes.iter().enumerate() {
        crate::require(
            route.edge_ids.len() == 2
                && route_offsets[route_rank].len() == 3
                && route_offsets[route_rank][1] == 5_130_000
                && route_offsets[route_rank][2] == 2 * 5_130_000,
            || format!("W 走廊须为两条 5 130 m 边的一维网格：route={route_rank}"),
        )?;
        crate::require(route.edge_ids[0] != route.edge_ids[1], || {
            format!("W 走廊两条边不得别名：route={route_rank}")
        })?;
        for id in &route.edge_ids {
            crate::require(seen_edges.insert(id.as_str()), || {
                format!("W 走廊不得与其他路线共享物理边：route={route_rank} edge={id}")
            })?;
        }
    }
    Ok(())
}

impl Sample {
    pub fn new(bundle: &Bundle, execution: ExecutionConfig) -> Result<Self, Error> {
        let input = check_canonical_network_input(
            bundle.files()["network.lfca"].as_slice(),
            FormatLimits::HARD,
        )
        .map_err(|e| failure("LFCA", e))?;
        let revision = build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::RetainAvailable,
                SharedNetworkBuildLimits::new(2 * 1_024 * 1_024 * 1_024, 2 * 1_024 * 1_024 * 1_024),
            ),
        )
        .map_err(|e| failure("shared revision", e))?;
        let state: InitialState = serde_json::from_slice(&bundle.files()["state.json"])?;
        let identity = revision.identity();
        let (profile, max_accel_mm_s2, emergency_decel_mm_s2) = profile_limits(&revision, &state)?;
        let origin = *revision.canonical_origin();
        let source = PublishedLfcaReference::new(
            "fixture://lf-synth-harness",
            origin.canonical_artifact_digest(),
            origin.canonical_artifact_byte_length(),
            origin.network_revision(),
        )
        .map_err(|e| failure("source", e))?;
        // 换道族路线（>2 边）可能为多簇（#1233）：保留首簇下标供单簇语义路径，
        // 同时给出全部簇下标用于生成开放入口。
        let lane_change_routes: Vec<usize> = state
            .routes
            .iter()
            .enumerate()
            .filter(|(_, route)| route.edge_ids.len() > 2)
            .map(|(index, _)| index)
            .collect();
        let lane_change_route = lane_change_routes.first().copied();
        let config = world_config(&state)?;
        let mut world = TrafficWorld::install(laneflow_runtime::WorldInstallInput {
            revision: revision.clone(),
            config,
            execution,
            source: CommittedNetworkSource::Published { reference: source },
            world_id: laneflow_runtime::WorldId::new(0),
            policy_selection: WorldPolicySelection::Pinned(PolicyPin {
                policy: state
                    .policy_id
                    .parse()
                    .map_err(|e| failure("policy id", e))?,
            }),
        })
        .map_err(|e| failure("install", e))?;
        let mut routes = Vec::new();
        let mut groups = Vec::new();
        let mut gates = Vec::new();
        for route in &state.routes {
            let edges = route
                .edge_ids
                .iter()
                .map(|id| {
                    let id = id
                        .parse::<LaneEdgeId>()
                        .map_err(|e| failure("edge id", e))?;
                    identity
                        .ordinal(id)
                        .ok_or_else(|| failure("edge", "missing"))
                })
                .collect::<Result<Vec<_>, Error>>()?;
            let (group, gate) = match &route.signal_group_id {
                Some(group_id) => {
                    let group = identity
                        .ordinal(
                            group_id
                                .parse::<SignalGroupId>()
                                .map_err(|e| failure("group id", e))?,
                        )
                        .ok_or_else(|| failure("group", "missing"))?;
                    let group_view = revision
                        .traffic()
                        .relations()
                        .signal_group(group)
                        .ok_or_else(|| failure("group relations", "missing"))?;
                    crate::require(group_view.gates().len() == 1, || {
                        "每条路线必须有唯一信号 Gate".into()
                    })?;
                    (Some(group), Some(group_view.gates()[0]))
                }
                // L 族无信号准入路线（换道簇/背景）：无信号组与 Gate。
                None => (None, None),
            };
            gates.push(gate);
            routes.push(
                world
                    .register_route(RouteRegisterInput::new(edges))
                    .map_err(|e| failure("route", e))?,
            );
            groups.push(group);
        }
        let mut handles = Vec::new();
        for vehicle in &state.vehicles {
            let route = routes[vehicle.route_rank as usize];
            let on_lane_change_route = lane_change_routes.contains(&(vehicle.route_rank as usize));
            let handle = if vehicle.status == "active" {
                let spawn = VehicleSpawnInput::new(
                    profile,
                    route,
                    vehicle.edge_occurrence,
                    vehicle.progress_mm,
                    vehicle.speed_mm_per_s,
                );
                world
                    .spawn_vehicle(if on_lane_change_route {
                        // 换道者车尾可越过路线起点（R1b 钉版后车 e0@3000）。
                        spawn.with_open_entrance()
                    } else {
                        spawn
                    })
                    .map_err(|e| failure("spawn", e))?
            } else {
                let space = identity
                    .ordinal(
                        vehicle
                            .parking_space_id
                            .as_deref()
                            .ok_or_else(|| failure("parking id", "missing"))?
                            .parse::<ParkingSpaceId>()
                            .map_err(|e| failure("parking id", e))?,
                    )
                    .ok_or_else(|| failure("parking space", "missing"))?;
                world
                    .spawn_parked_vehicle(
                        ParkedVehicleSpawnInput::new(
                            profile,
                            route,
                            vehicle.edge_occurrence,
                            vehicle.progress_mm,
                        ),
                        ParkingTarget::ExplicitSpace(space),
                    )
                    .map_err(|e| failure("parked spawn", e))?
                    .vehicle
            };
            let actual = world
                .vehicle(handle)
                .ok_or_else(|| failure("spawn state", "missing"))?;
            crate::require(
                actual.profile() == profile
                    && actual.route() == route
                    && actual.route_edge_index() == vehicle.edge_occurrence
                    && actual.progress_mm() == vehicle.progress_mm
                    && actual.speed_mm_s() == vehicle.speed_mm_per_s
                    && actual.carry_um() == 0
                    && actual.status()
                        == if vehicle.status == "active" {
                            VehicleStatus::Active
                        } else {
                            VehicleStatus::Parked
                        },
                || format!("初始状态与目录不符 rank={}", vehicle.logical_rank),
            )?;
            handles.push(handle);
        }
        let initial: Vec<_> = handles.iter().map(|&h| world.vehicle(h).unwrap()).collect();
        let bindings = handles.iter().map(|&h| world.parking_binding(h)).collect();
        let cursor = world.command_cursor();
        let route_offsets = route_position_offsets(&state, &revision)?;
        validate_w_signal_reference_domain(
            &revision,
            &state,
            &route_offsets,
            emergency_decel_mm_s2,
        )?;
        validate_w_corridor_binding(&state, &route_offsets)?;
        let lane_change_slots = state
            .identity
            .lane_change()
            .map_or(0, |params| params.clusters * params.changers_per_cluster)
            as usize;
        Ok(Self {
            world,
            state,
            handles,
            routes,
            groups,
            gates,
            lane_change_route,
            lane_change_routes,
            lane_change_journal: Vec::new(),
            lane_change_last_kind: vec![0; lane_change_slots],
            route_offsets,
            previous: initial.clone(),
            crossed: vec![false; initial.len()],
            trace_bytes: Vec::with_capacity(initial.len() * 32),
            initial,
            bindings,
            transitions: 0,
            deceleration: crate::decel::Counters::default(),
            max_accel_mm_s2,
            emergency_decel_mm_s2,
            cursor,
            session: None,
            input_identity: String::new(),
        })
    }
}

/// 世界容量（#1218 L 族适配）：L 族车辆容量含两换道者、边出现记录按路线实长累计、
/// 冲突出现记录预算 16（换道簇 T 出现记录）；W 族恒为（规模、路线数×2、0），逐字保持。
/// 恢复重建必须与安装同式（否则 `restore_lfrs` 容量校验拒绝）。
pub(crate) fn world_config(state: &InitialState) -> Result<WorldConfig, Error> {
    let mut input = WorldConfigInput {
        vehicle_capacity: state.identity.scale,
        route_capacity: state.routes.len() as u32,
        route_edge_occurrence_capacity: state.routes.len() as u64 * 2,
        route_conflict_occurrence_capacity: 0,
        fixed_delta_time_ms: u64::from(state.identity.fixed_delta_ms),
    };
    let cluster_routes = state
        .routes
        .iter()
        .filter(|route| route.edge_ids.len() > 2)
        .count() as u64;
    if cluster_routes > 0 {
        input.vehicle_capacity = state.vehicles.len() as u32;
        input.route_edge_occurrence_capacity = state
            .routes
            .iter()
            .map(|route| route.edge_ids.len() as u64)
            .sum();
        // 每簇路线注册 2 个冲突出现记录；容量进入快照摘要，保留历史下界
        // 16（#1233：c1/c4 既有证据不变，16 簇起按需扩展）。
        input.route_conflict_occurrence_capacity = cluster_routes.saturating_mul(2).max(16);
    }
    WorldConfig::try_new(input).map_err(|error| failure("world config", error))
}

/// 逐路线真实边长前缀和（毫米）：位置编码不假设边长均匀（#1218 L 族）。
pub(crate) fn route_position_offsets(
    state: &InitialState,
    revision: &SharedNetworkRevision,
) -> Result<Vec<Vec<u64>>, Error> {
    let lengths = revision.traffic().lane_lengths_millimetres();
    let mut offsets = Vec::with_capacity(state.routes.len());
    for route in &state.routes {
        let mut row = Vec::with_capacity(route.edge_ids.len() + 1);
        let mut accumulated = 0_u64;
        row.push(0);
        for id in &route.edge_ids {
            let id = id
                .parse::<LaneEdgeId>()
                .map_err(|e| failure("edge id", e))?;
            let ordinal = revision
                .identity()
                .ordinal(id)
                .ok_or_else(|| failure("edge", "missing"))?;
            accumulated += u64::from(lengths[ordinal.raw() as usize]);
            row.push(accumulated);
        }
        offsets.push(row);
    }
    Ok(offsets)
}
