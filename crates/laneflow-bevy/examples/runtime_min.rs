//! 最小 Bevy 示例：`LaneFlowPlugin` + `LaneFlowSession` 驱动代理位移。
//!
//! GUI 不进 CI。CI 通过 `tests/runtime_min_smoke.rs` 跑无窗口 App。

use std::{error::Error, num::NonZeroU32, sync::Arc};

use bevy::prelude::*;
use laneflow_bevy::{LaneFlowPlugin, LaneFlowSession, LaneFlowSessionConfig, pose_input};
use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{
    CommittedNetworkSource, PublishedLfcaReference, RouteRegisterInput, TrafficWorld,
    VehicleSpawnInput, WorldConfig,
};
use laneflow_spatial::{CanonicalPoseBatch, FramePlacementToken, PoseRecordId, SpatialSession};
use laneflow_static_contract::{LaneEdgeOrdinal, VehicleProfileOrdinal};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};

const FULL_SPATIAL: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfca"
);

fn main() -> Result<(), Box<dyn Error>> {
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
    let mut world = {
        let origin = revision.canonical_origin();
        TrafficWorld::install(
            Arc::clone(&revision),
            WorldConfig::new(8, 4, 1, 100),
            CommittedNetworkSource::Published {
                reference: PublishedLfcaReference::new(
                    "scenario://runtime-min",
                    origin.canonical_artifact_digest(),
                    origin.canonical_artifact_byte_length(),
                    origin.network_revision(),
                )
                .expect("non-empty scenario key"),
            },
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
    let session = LaneFlowSession::new(
        world,
        Some(spatial),
        LaneFlowSessionConfig::new(NonZeroU32::new(8).expect("non-zero")),
    )?;

    App::new()
        .add_plugins((DefaultPlugins, LaneFlowPlugin))
        .insert_resource(session)
        .add_systems(Startup, spawn_proxy)
        .add_systems(Update, sync_proxy)
        .run();
    Ok(())
}

#[derive(Resource)]
struct Proxy(Entity);

fn spawn_proxy(mut commands: Commands) {
    let entity = commands.spawn(Transform::IDENTITY).id();
    commands.insert_resource(Proxy(entity));
}

fn sync_proxy(
    mut session: ResMut<LaneFlowSession>,
    proxy: Option<Res<Proxy>>,
    mut transforms: Query<&mut Transform>,
) {
    let Some(proxy) = proxy else {
        return;
    };
    let poses = session.world().committed_pose_sources();
    let inputs: Vec<_> = poses
        .as_slice()
        .iter()
        .enumerate()
        .map(|(index, (_, source))| pose_input(PoseRecordId::new(index as u32), *source))
        .collect();
    let Some(spatial) = session.spatial_mut() else {
        return;
    };
    let mut batch = CanonicalPoseBatch::new();
    if spatial
        .extract_pose_batch(FramePlacementToken::new(1), &inputs, &mut batch)
        .is_err()
    {
        return;
    }
    let Some(record) = batch.records().first() else {
        return;
    };
    if let Ok(mut transform) = transforms.get_mut(proxy.0) {
        let position = record.pose().position();
        *transform = Transform::from_xyz(position.x(), position.y(), position.z());
    }
}
