//! runtime_min 与无窗口 smoke 共用的完整初始化路径。
use std::{error::Error, num::NonZeroU32, sync::Arc};

use laneflow_bevy::{LaneFlowSession, LaneFlowSessionConfig};
use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{
    CommittedNetworkSource, PolicyPin, PublishedLfcaReference, RouteRegisterInput, TrafficWorld,
    VehicleSpawnInput, WorldConfig, WorldPolicySelection,
};
use laneflow_spatial::SpatialSession;
use laneflow_static_contract::{
    EntityKind, LaneEdgeOrdinal, RightOfWayPolicySetId, VehicleProfileOrdinal,
};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};

const FULL_SPATIAL: &[u8] = include_bytes!(
    "../../../laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfca"
);

pub fn session() -> Result<LaneFlowSession, Box<dyn Error>> {
    let input = check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD)
        .map_err(|error| format!("{error:?}"))?;
    let revision = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .map_err(|error| format!("{error:?}"))?;
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
    let mut world = {
        let origin = revision.canonical_origin();
        TrafficWorld::install(
            Arc::clone(&revision),
            WorldConfig::new(8, 4, 1_024, 1_024, 1, 100),
            CommittedNetworkSource::Published {
                reference: PublishedLfcaReference::new(
                    "scenario://runtime-min",
                    origin.canonical_artifact_digest(),
                    origin.canonical_artifact_byte_length(),
                    origin.network_revision(),
                )
                .expect("non-empty scenario key"),
            },
            0,
            WorldPolicySelection::Pinned(PolicyPin { policy }),
        )?
    };
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
    world.spawn_vehicle(VehicleSpawnInput::new(
        VehicleProfileOrdinal::from_raw(0),
        route,
        0,
        1_000 + profile.length_mm() + profile.min_gap_mm() + 2_000,
        0,
    ))?;
    world.spawn_vehicle(VehicleSpawnInput::new(
        VehicleProfileOrdinal::from_raw(0),
        route,
        0,
        1_000,
        0,
    ))?;
    let spatial = SpatialSession::bind(revision)
        .map_err(|error| format!("{error:?}"))?
        .ok_or("missing spatial session")?;
    Ok(LaneFlowSession::new(
        world,
        Some(spatial),
        LaneFlowSessionConfig::new(NonZeroU32::new(8).expect("non-zero")),
    )?)
}
