//! 薄走廊 Bevy 路径：检入的走廊 LFCA + 少数车辆 tick / pose。
//!
//! 不恢复 50–200 人口、HUD、灯具或同一 Entity 回流。GUI 不进 CI。

use std::{error::Error, num::NonZeroU32, sync::Arc};

use bevy::prelude::*;
use laneflow_bevy::{LaneFlowPlugin, LaneFlowSession, LaneFlowSessionConfig, pose_input};
use laneflow_format::{FormatLimits, check_canonical_network_input_v1};
use laneflow_runtime::{TrafficWorld, VehicleSpawnInput, WorldConfig};
use laneflow_spatial::{CanonicalPoseBatch, FramePlacementToken, PoseRecordId, SpatialSession};
use laneflow_static_contract::{StaticRouteOrdinal, VehicleProfileOrdinal};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};

const CORRIDOR_LFCA: &[u8] = include_bytes!("../../../examples/data/v0.2-signalized-corridor.lfca");

fn main() -> Result<(), Box<dyn Error>> {
    let input = check_canonical_network_input_v1(CORRIDOR_LFCA, FormatLimits::V1_HARD)
        .map_err(|error| format!("{error:?}"))?;
    let revision = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .map_err(|error| format!("{error:?}"))?;
    let mut world = TrafficWorld::install(Arc::clone(&revision), WorldConfig::new(8, 8, 1, 16))?;
    let route = world.static_route(StaticRouteOrdinal::from_raw(0))?;
    let profile = world
        .traffic()
        .relations()
        .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
        .ok_or("missing profile")?;
    world.spawn_vehicle(VehicleSpawnInput::new(
        VehicleProfileOrdinal::from_raw(0),
        route,
        0,
        1.0 + profile.length() + profile.min_gap() + 2.0,
        0.0,
    ))?;
    world.spawn_vehicle(VehicleSpawnInput::new(
        VehicleProfileOrdinal::from_raw(0),
        route,
        0,
        1.0,
        0.0,
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
