//! `junction_debug` 示例与无窗口 smoke 共用的复杂路口场景装配（#285 阶段二）。
//!
//! 从检入的 catalog 0.1 + LFCA 建立 Session：Identity v1 绑定 → 安装世界 →
//! 注册全部 catalog 路线 → 固定四车 spawn 计划。无 PRNG，两次装配逐状态一致。

use std::{error::Error, num::NonZeroU32, sync::Arc};

use laneflow_bevy::{LaneFlowSession, LaneFlowSessionConfig};
use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{
    CommittedNetworkSource, PublishedLfcaReference, RouteHandle, TrafficWorld, VehicleHandle,
    VehicleSpawnInput, WorldConfig,
};
use laneflow_scenario::complex_junction::{JunctionCatalog, VEHICLE_PROFILE_KEY, bind};
use laneflow_spatial::SpatialSession;
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};

const JUNCTION_LFCA: &[u8] = include_bytes!("../../../../examples/data/v0.1-complex-junction.lfca");
const JUNCTION_CATALOG: &str =
    include_str!("../../../../examples/data/v0.1-complex-junction.catalog.toml");

/// 固定 dt（毫秒），与场景合同 `examples/config/v0.1-complex-junction.toml` 的
/// `fixed_delta_ms` 一致。
const FIXED_DELTA_MS: u64 = 16;

/// 四车 spawn 计划的角色：直行车、进待转区的保护左转车、先 NoGrant 后找间隙的
/// 许可左转车、环路重复过门车。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VehicleRole {
    Through,
    WaitingLeft,
    PermissiveLeft,
    Circuit,
}

impl VehicleRole {
    /// 面板展示的 ASCII 角色标签（bevy 内嵌字体仅覆盖 ASCII）。
    pub const fn tag(self) -> &'static str {
        match self {
            Self::Through => "through",
            Self::WaitingLeft => "waiting-left",
            Self::PermissiveLeft => "permissive-left",
            Self::Circuit => "circuit",
        }
    }
}

/// 一辆车 spawn 计划条目：角色 + catalog route/slot 身份。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpawnPlanEntry {
    pub role: VehicleRole,
    pub route_id: &'static str,
    pub slot_id: &'static str,
}

/// 确定性 spawn 计划（无 PRNG）：进度与车型全部取自 catalog spawn slot 与
/// catalog 唯一车型 profile。槽位取环路末段（进度约 306.5 m），让四辆车在
/// 一个信号周期内到达机动门与冲突区，调试 overlay 与 smoke 不必空转整圈环路。
pub const SPAWN_PLAN: [SpawnPlanEntry; 4] = [
    SpawnPlanEntry {
        role: VehicleRole::Through,
        route_id: "route-w-through",
        slot_id: "slot-loop-sw-i1-030",
    },
    SpawnPlanEntry {
        role: VehicleRole::WaitingLeft,
        route_id: "route-w-left-waiting",
        slot_id: "slot-loop-sw-i0-030",
    },
    SpawnPlanEntry {
        role: VehicleRole::PermissiveLeft,
        route_id: "route-n-permissive-left",
        slot_id: "slot-loop-wn-i0-030",
    },
    SpawnPlanEntry {
        role: VehicleRole::Circuit,
        route_id: "route-n-left-circuit",
        slot_id: "slot-loop-wn-i1-030",
    },
];

/// 已装配世界里的一辆 spawn 车：宿主侧表现绑定所需的全部身份。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpawnedVehicle {
    pub role: VehicleRole,
    pub vehicle: VehicleHandle,
    pub route: RouteHandle,
    pub route_id: &'static str,
    pub slot_id: &'static str,
}

/// 装配完成的复杂路口调试场景。
pub struct JunctionDebugScene {
    pub session: LaneFlowSession,
    pub spawned: Box<[SpawnedVehicle]>,
}

pub fn session_config() -> LaneFlowSessionConfig {
    LaneFlowSessionConfig::new(NonZeroU32::new(8).expect("non-zero"))
}

/// 从检入制品装配 Session 并执行固定 spawn 计划。
pub fn build() -> Result<JunctionDebugScene, Box<dyn Error>> {
    let catalog: JunctionCatalog = toml::from_str(JUNCTION_CATALOG)?;
    let input = check_canonical_network_input(JUNCTION_LFCA, FormatLimits::HARD)
        .map_err(|error| format!("{error:?}"))?;
    let revision = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .map_err(|error| format!("{error:?}"))?;
    let bound = bind(&catalog, &revision).map_err(|error| error.to_string())?;
    let mut world = {
        let origin = revision.canonical_origin();
        TrafficWorld::install(
            Arc::clone(&revision),
            WorldConfig::new(16, 16, 1_024, 1_024, 1, FIXED_DELTA_MS),
            CommittedNetworkSource::Published {
                reference: PublishedLfcaReference::new(
                    "scenario://complex-junction",
                    origin.canonical_artifact_digest(),
                    origin.canonical_artifact_byte_length(),
                    origin.network_revision(),
                )
                .expect("non-empty scenario key"),
            },
            0,
            bound.policy_selection,
        )?
    };
    let profile = *bound
        .profiles
        .get(VEHICLE_PROFILE_KEY)
        .ok_or("missing standard-car profile")?;
    let routes = bound
        .install_routes(&mut world)
        .map_err(|error| error.to_string())?;
    let mut spawned = Vec::with_capacity(SPAWN_PLAN.len());
    for entry in &SPAWN_PLAN {
        let route_index = catalog
            .routes
            .iter()
            .position(|route| route.route_id == entry.route_id)
            .ok_or("spawn plan route missing from catalog")?;
        let route = *routes
            .get(route_index)
            .ok_or("catalog route must be registered")?;
        let slot = bound
            .spawn_slots
            .iter()
            .find(|slot| slot.slot_id == entry.slot_id)
            .ok_or("spawn plan slot missing from bound catalog")?;
        let vehicle = world.spawn_vehicle(VehicleSpawnInput::new(
            profile,
            route,
            0,
            slot.progress_mm,
            0,
        ))?;
        spawned.push(SpawnedVehicle {
            role: entry.role,
            vehicle,
            route,
            route_id: entry.route_id,
            slot_id: entry.slot_id,
        });
    }
    let spatial = SpatialSession::bind(revision)
        .map_err(|error| format!("{error:?}"))?
        .ok_or("missing spatial session")?;
    let session = LaneFlowSession::new(world, Some(spatial), session_config())?;
    Ok(JunctionDebugScene {
        session,
        spawned: spawned.into_boxed_slice(),
    })
}
