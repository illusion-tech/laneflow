//! 现行走廊 Bevy 最小路径：检入的 catalog 0.3 + LFCA，prepare 绑定后少数车辆 tick / pose。
//!
//! 不恢复 50–200 人口、HUD、灯具或同一 Entity 回流。GUI 不进 CI。

use std::{error::Error, num::NonZeroU32, sync::Arc};

use bevy::prelude::*;
use laneflow_bevy::{LaneFlowPlugin, LaneFlowSession, LaneFlowSessionConfig, pose_input};
use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{TrafficWorld, VehicleSpawnInput, WorldConfig};
use laneflow_scenario::signalized_corridor::{
    BoundCorridorCatalog, BoundSpawnSlot, CorridorCatalog, PASSENGER_CAR_PROFILE_KEY, bind,
};
use laneflow_spatial::{CanonicalPoseBatch, FramePlacementToken, PoseRecordId, SpatialSession};
use laneflow_static_contract::VehicleProfileOrdinal;
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};

const CORRIDOR_LFCA: &[u8] = include_bytes!("../../../examples/data/v0.2-signalized-corridor.lfca");
const CORRIDOR_CATALOG: &str =
    include_str!("../../../examples/data/v0.2-signalized-corridor.catalog.toml");

fn main() -> Result<(), Box<dyn Error>> {
    let catalog: CorridorCatalog = toml::from_str(CORRIDOR_CATALOG)?;
    let input = check_canonical_network_input(CORRIDOR_LFCA, FormatLimits::HARD)
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
    let mut world = TrafficWorld::install(Arc::clone(&revision), WorldConfig::new(8, 32, 1, 16))?;
    let profile = *bound
        .profiles
        .get(PASSENGER_CAR_PROFILE_KEY)
        .ok_or("missing passenger-car profile")?;
    let routes = bound
        .install_routes(&mut world)
        .map_err(|error| error.to_string())?;
    let (follower, leader) = follow_pair(&catalog, &bound)?;
    spawn_on_slot(&mut world, profile, leader, &routes)?;
    spawn_on_slot(&mut world, profile, follower, &routes)?;
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

fn follow_pair<'a>(
    catalog: &CorridorCatalog,
    bound: &'a BoundCorridorCatalog,
) -> Result<(&'a BoundSpawnSlot, &'a BoundSpawnSlot), Box<dyn Error>> {
    let lane = catalog
        .portals
        .first()
        .and_then(|portal| portal.lanes.first())
        .ok_or("missing portal lane")?;
    let follower = bound
        .spawn_slots
        .iter()
        .find(|slot| slot.slot_id == lane.entry_spawn_slot_id)
        .ok_or("missing entry spawn slot")?;
    let leader = bound
        .spawn_slots
        .iter()
        .find(|slot| {
            slot.portal_id == follower.portal_id
                && slot.lane_index == follower.lane_index
                && slot.edge == follower.edge
                && slot.progress_mm > follower.progress_mm
        })
        .ok_or("missing leader spawn slot")?;
    Ok((follower, leader))
}

fn proxy_transform(pose: laneflow_spatial::CanonicalPoseF32) -> Transform {
    let position = pose.position();
    let tangent = pose.tangent();
    let up = pose.up();
    Transform::from_xyz(position.x(), position.y(), position.z()).looking_to(
        Vec3::new(tangent.x(), tangent.y(), tangent.z()),
        Vec3::new(up.x(), up.y(), up.z()),
    )
}

fn spawn_on_slot(
    world: &mut TrafficWorld,
    profile: VehicleProfileOrdinal,
    slot: &BoundSpawnSlot,
    routes: &[laneflow_runtime::RouteHandle],
) -> Result<(), Box<dyn Error>> {
    let index = slot
        .entry_edges
        .iter()
        .position(|edge| *edge == slot.edge)
        .ok_or("slot edge is not on its entry route")?;
    let route = *routes
        .get(slot.route_index)
        .ok_or("catalog route must be registered")?;
    world.spawn_vehicle(VehicleSpawnInput::new(
        profile,
        route,
        u32::try_from(index)?,
        slot.progress_mm,
        0,
    ))?;
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
        *transform = proxy_transform(record.pose());
    }
}
