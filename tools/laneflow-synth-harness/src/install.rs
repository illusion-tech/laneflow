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
    pub lane_change_journal: Vec<(u64, u8, u32, u32)>,
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
    pub max_deceleration_mm_s2: u64,
    pub deceleration_above_profile_count: u64,
    /// 车型最大加速度（毫米/秒²）：由共享路网 profile 视图派生，运动学语义
    /// 检查按车型上界判定（W 族 1.4、L 族 1.5，#1218）。
    pub max_accel_mm_s2: u32,
    pub cursor: u64,
    pub trace_bytes: Vec<u8>,
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
        let profile = identity
            .ordinal(
                state
                    .vehicle_profile_id
                    .parse::<VehicleProfileId>()
                    .map_err(|e| failure("profile id", e))?,
            )
            .ok_or_else(|| failure("profile", "missing"))?;
        let max_accel_mm_s2 = revision
            .traffic()
            .relations()
            .vehicle_profile(profile)
            .ok_or_else(|| failure("profile view", "missing"))?
            .max_accel();
        let max_accel_mm_s2 = (f64::from(max_accel_mm_s2) * 1_000.0).round().max(0.0) as u32;
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
        let config = world_config(&state);
        let mut world = TrafficWorld::install(
            revision.clone(),
            config,
            execution,
            CommittedNetworkSource::Published { reference: source },
            0,
            WorldPolicySelection::Pinned(PolicyPin {
                policy: state
                    .policy_id
                    .parse()
                    .map_err(|e| failure("policy id", e))?,
            }),
        )
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
            max_deceleration_mm_s2: 0,
            deceleration_above_profile_count: 0,
            max_accel_mm_s2,
            cursor,
        })
    }
}

/// 世界容量（#1218 L 族适配）：L 族车辆容量含两换道者、边出现记录按路线实长累计、
/// 冲突出现记录预算 16（换道簇 T 出现记录）；W 族恒为（规模、路线数×2、0），逐字保持。
/// 恢复重建必须与安装同式（否则 `restore_lfrs` 容量校验拒绝）。
pub(crate) fn world_config(state: &InitialState) -> WorldConfig {
    if state.routes.iter().any(|route| route.edge_ids.len() > 2) {
        let cluster_routes = state
            .routes
            .iter()
            .filter(|route| route.edge_ids.len() > 2)
            .count() as u64;
        // 每簇路线注册 2 个冲突出现记录；容量进入快照摘要，保留历史下界
        // 16（#1233：c1/c4 既有证据不变，16 簇起按需扩展）。
        let conflict_capacity = cluster_routes.saturating_mul(2).max(16);
        WorldConfig::new(
            state.vehicles.len() as u32,
            state.routes.len() as u32,
            state.routes.iter().map(|r| r.edge_ids.len() as u64).sum(),
            conflict_capacity,
            u64::from(state.identity.fixed_delta_ms),
        )
    } else {
        WorldConfig::new(
            state.identity.scale,
            state.routes.len() as u32,
            state.routes.len() as u64 * 2,
            0,
            u64::from(state.identity.fixed_delta_ms),
        )
    }
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
