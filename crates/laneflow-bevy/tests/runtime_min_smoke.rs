use std::{num::NonZeroU32, sync::Arc, time::Duration};

use bevy_app::App;
use bevy_ecs::{
    entity::Entity,
    resource::Resource,
    system::{Commands, Query, Res, ResMut},
};
use bevy_time::{TimePlugin, TimeUpdateStrategy};
use bevy_transform::{TransformPlugin, components::Transform};
use laneflow_bevy::{LaneFlowPlugin, LaneFlowSession, LaneFlowSessionConfig, pose_input};
use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{RouteRegisterInput, TrafficWorld, VehicleSpawnInput, WorldConfig};
use laneflow_spatial::{CanonicalPoseBatch, FramePlacementToken, PoseRecordId, SpatialSession};
use laneflow_static_contract::{LaneEdgeOrdinal, VehicleProfileOrdinal};
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

const FULL_SPATIAL: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfca"
);

#[derive(Resource)]
struct Proxy(Entity);

fn revision() -> Arc<laneflow_static_network::SharedNetworkRevision> {
    let input = check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD)
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

fn edge_for_length(world: &TrafficWorld, length: u32) -> LaneEdgeOrdinal {
    let index = world
        .traffic()
        .lane_lengths_millimetres()
        .iter()
        .position(|actual| *actual == length)
        .expect("fixture lane length");
    LaneEdgeOrdinal::try_from_usize(index).expect("fixture lane ordinal")
}

fn spawn_two_vehicles(world: &mut TrafficWorld) {
    let route = world
        .register_route(RouteRegisterInput::new(vec![
            edge_for_length(world, 10_000),
            edge_for_length(world, 8_000),
            edge_for_length(world, 12_000),
        ]))
        .expect("register");
    let profile = world
        .traffic()
        .relations()
        .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
        .expect("profile");
    world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            1_000 + profile.length_mm() + profile.min_gap_mm() + 2_000,
            0,
        ))
        .expect("leader");
    world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            1_000,
            0,
        ))
        .expect("follower");
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
        let position = record.pose().position();
        *transform = Transform::from_xyz(position.x(), position.y(), position.z());
    }
}

#[test]
fn headless_app_steps_runtime_and_moves_proxy_transform() {
    let revision = revision();
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(8, 4, 1_024, 1_024, 1, 100),
    )
    .expect("install");
    spawn_two_vehicles(&mut world);
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
        100,
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
    assert_ne!(
        [
            before.translation.x,
            before.translation.y,
            before.translation.z
        ],
        [
            after.translation.x,
            after.translation.y,
            after.translation.z
        ],
        "proxy Transform must change after runtime steps"
    );
}
