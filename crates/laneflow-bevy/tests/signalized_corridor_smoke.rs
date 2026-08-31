use std::{num::NonZeroU32, sync::Arc, time::Duration};

use bevy_app::App;
use bevy_ecs::{
    entity::Entity,
    resource::Resource,
    system::{Commands, Query, Res, ResMut},
};
use bevy_math::Vec3;
use bevy_time::{TimePlugin, TimeUpdateStrategy};
use bevy_transform::{TransformPlugin, components::Transform};
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

fn install_fixture(
    revision: std::sync::Arc<laneflow_static_network::SharedNetworkRevision>,
    config: laneflow_runtime::WorldConfig,
) -> Result<laneflow_runtime::TrafficWorld, laneflow_runtime::InstallError> {
    let origin = *revision.canonical_origin();
    laneflow_runtime::TrafficWorld::install(
        revision,
        config,
        laneflow_runtime::CommittedNetworkSource::Published {
            reference: laneflow_runtime::PublishedLfcaReference::new(
                "fixture://in-process",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("non-empty fixture key"),
        },
        0,
    )
}

const CORRIDOR_LFCA: &[u8] = include_bytes!("../../../examples/data/v0.2-signalized-corridor.lfca");
const CORRIDOR_CATALOG: &str =
    include_str!("../../../examples/data/v0.2-signalized-corridor.catalog.toml");

#[derive(Resource)]
struct Proxy(Entity);

fn revision() -> Arc<laneflow_static_network::SharedNetworkRevision> {
    let input = check_canonical_network_input(CORRIDOR_LFCA, FormatLimits::HARD)
        .expect("checked canonical network input");
    build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .expect("shared network revision")
}

fn spawn_on_slot(
    world: &mut TrafficWorld,
    profile: VehicleProfileOrdinal,
    slot: &BoundSpawnSlot,
    routes: &[laneflow_runtime::RouteHandle],
) {
    let route = *routes
        .get(slot.route_index)
        .expect("catalog route must be registered");
    world
        .spawn_vehicle(VehicleSpawnInput::new(
            profile,
            route,
            0,
            slot.progress_mm,
            0,
        ))
        .expect("catalog slot must spawn");
}

fn spawn_two_vehicles(
    world: &mut TrafficWorld,
    revision: &laneflow_static_network::SharedNetworkRevision,
) {
    let catalog: CorridorCatalog = toml::from_str(CORRIDOR_CATALOG).expect("catalog TOML");
    let bound = bind(&catalog, revision).expect("prepare bind");
    assert_eq!(bound.network_revision, revision.network_revision());
    let routes = bound.install_routes(world).expect("install catalog routes");
    let profile = *bound
        .profiles
        .get(PASSENGER_CAR_PROFILE_KEY)
        .expect("passenger-car profile");
    let (follower, leader) = follow_pair(&catalog, &bound);
    spawn_on_slot(world, profile, leader, &routes);
    spawn_on_slot(world, profile, follower, &routes);
}

fn follow_pair<'a>(
    catalog: &CorridorCatalog,
    bound: &'a BoundCorridorCatalog,
) -> (&'a BoundSpawnSlot, &'a BoundSpawnSlot) {
    let lane = catalog
        .portals
        .first()
        .and_then(|portal| portal.lanes.first())
        .expect("portal lane");
    let follower = bound
        .spawn_slots
        .iter()
        .find(|slot| slot.slot_id == lane.entry_spawn_slot_id)
        .expect("entry spawn slot");
    let leader = bound
        .spawn_slots
        .iter()
        .find(|slot| {
            slot.portal_id == follower.portal_id
                && slot.lane_index == follower.lane_index
                && slot.edge == follower.edge
                && slot.progress_mm > follower.progress_mm
        })
        .expect("leader spawn slot");
    (follower, leader)
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

fn setup_proxy(mut commands: Commands) {
    let entity = commands.spawn(Transform::IDENTITY).id();
    commands.insert_resource(Proxy(entity));
}

fn sync_proxy(
    mut session: ResMut<LaneFlowSession>,
    proxy: Option<Res<Proxy>>,
    mut transforms: Query<&mut Transform>,
) {
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
    spatial
        .extract_pose_batch(FramePlacementToken::new(1), &inputs, &mut batch)
        .expect("extract");
    let Some(record) = batch.records().first() else {
        return;
    };
    let Some(proxy) = proxy else {
        return;
    };
    if let Ok(mut transform) = transforms.get_mut(proxy.0) {
        *transform = proxy_transform(record.pose());
    }
}

#[test]
fn headless_app_steps_corridor_runtime_and_moves_proxy_transform() {
    let revision = revision();
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(8, 32, 1_024, 1_024, 1, 16),
    )
    .expect("install");
    spawn_two_vehicles(&mut world, &revision);
    let spatial = SpatialSession::bind(revision)
        .expect("bind")
        .expect("session");
    let session = LaneFlowSession::new(
        world,
        Some(spatial),
        LaneFlowSessionConfig::new(NonZeroU32::new(8).expect("non-zero")),
    )
    .expect("paired session");
    let mut app = App::new();
    app.add_plugins((TimePlugin, TransformPlugin, LaneFlowPlugin));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        16,
    )));
    app.insert_resource(session);
    app.add_systems(bevy_app::Startup, setup_proxy);
    app.add_systems(bevy_app::Update, sync_proxy);
    app.update();
    let before = {
        let entity = app.world().resource::<Proxy>().0;
        *app.world().get::<Transform>(entity).expect("transform")
    };
    for _ in 0..16 {
        app.update();
    }
    assert!(
        app.world()
            .resource::<LaneFlowSession>()
            .frame_report()
            .steps_run()
            > 0,
        "LaneFlowFixed schedule must step TrafficWorld"
    );
    let after = {
        let entity = app.world().resource::<Proxy>().0;
        *app.world().get::<Transform>(entity).expect("transform")
    };
    assert_ne!(before.translation, after.translation);
}
