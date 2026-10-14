//! runtime_min 与无窗口 smoke 共用的完整初始化路径。
use std::{error::Error, num::NonZeroU32};

use laneflow_bevy::LaneFlowSession;
use laneflow_runtime::{
    PolicyPin, RouteRegisterInput, TrafficWorld, VehicleSpawnInput, WorldConfig,
    WorldPolicySelection,
};
use laneflow_static_contract::{
    EntityKind, LaneEdgeOrdinal, RightOfWayPolicySetId, VehicleProfileOrdinal,
};

#[path = "scene_loader.rs"]
mod scene_loader;

const FULL_SPATIAL: &[u8] = include_bytes!(
    "../../../laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfca"
);

const MAX_CATCH_UP_STEPS: NonZeroU32 = NonZeroU32::new(8).expect("non-zero");

/// 安装夹具修订、注册沿机动路径的路线并生成两辆车的世界。
pub fn world() -> Result<TrafficWorld, Box<dyn Error>> {
    let revision = scene_loader::shared_revision(FULL_SPATIAL)?;
    // 宿主选择夹具明确声明的策略身份，不按根内策略顺序或缺省值选取。
    let policy = RightOfWayPolicySetId::from_untyped(
        laneflow_compiler::derive_canonical_stable_id_v1(
            EntityKind::RightOfWayPolicySet,
            "runtime-fixture-policy",
            "fixture-policy",
            &laneflow_compiler::CompileLimits::p100_initial_v1(),
        )
        .map_err(|error| format!("{error:?}"))?,
    );
    let mut world = scene_loader::install_world(
        &revision,
        "scenario://runtime-min",
        WorldConfig::new(8, 4, 1_024, 1_024, 100),
        WorldPolicySelection::Pinned(PolicyPin { policy }),
    )?;
    let edge_for_length = |world: &TrafficWorld, length: u32| {
        let index = world
            .traffic()
            .lane_lengths_millimetres()
            .iter()
            .position(|actual| *actual == length)
            .expect("fixture lane length");
        LaneEdgeOrdinal::try_from_usize(index).expect("fixture lane ordinal")
    };
    let route = world.register_route(RouteRegisterInput::new(vec![
        edge_for_length(&world, 10_000),
        edge_for_length(&world, 8_000),
        edge_for_length(&world, 12_000),
    ]))?;
    let profile = world
        .traffic()
        .relations()
        .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
        .ok_or("missing profile")?;
    world.spawn_vehicle(
        VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            1_000 + profile.length_mm() + profile.min_gap_mm() + 2_000,
            0,
        )
        .with_open_entrance(),
    )?;
    world.spawn_vehicle(
        VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 1_000, 0)
            .with_open_entrance(),
    )?;
    Ok(world)
}

pub fn session() -> Result<LaneFlowSession, Box<dyn Error>> {
    scene_loader::paired_session(world()?, MAX_CATCH_UP_STEPS)
}
