use std::sync::Arc;

use bevy_app::App;
use bevy_ecs::{
    entity::Entity,
    resource::Resource,
    system::{Commands, Query, ResMut},
};
use bevy_time::TimePlugin;
use bevy_transform::{TransformPlugin, components::Transform};
use laneflow_format::{FormatLimits, check_canonical_network_input_v1};
use laneflow_runtime::{PoseSource, TickInput, TrafficWorld, VehicleSpawnInput, WorldConfig};
use laneflow_spatial::{
    CanonicalPoseBatch, FramePlacementToken, PoseInput, PoseRecordId, SpatialSession,
};
use laneflow_static_contract::{StaticRouteOrdinal, VehicleProfileOrdinal};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};

const FULL_SPATIAL: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable-v1/lfca-v1-full-spatial/expected.lfca"
);

#[derive(Resource)]
struct MinRuntime {
    world: TrafficWorld,
    session: SpatialSession,
    proxy: Option<Entity>,
}

fn revision() -> Arc<laneflow_static_network::SharedNetworkRevision> {
    let input = check_canonical_network_input_v1(FULL_SPATIAL, FormatLimits::V1_HARD)
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

fn spawn_two_vehicles(world: &mut TrafficWorld) {
    let route = world
        .static_route(StaticRouteOrdinal::from_raw(0))
        .expect("static route");
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
            1.0 + profile.length() + profile.min_gap() + 2.0,
            0.0,
        ))
        .expect("leader");
    world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            1.0,
            0.0,
        ))
        .expect("follower");
}

fn setup_proxy(mut commands: Commands, mut runtime: ResMut<MinRuntime>) {
    runtime.proxy = Some(commands.spawn(Transform::IDENTITY).id());
}

fn step_runtime(mut runtime: ResMut<MinRuntime>, mut transforms: Query<&mut Transform>) {
    runtime.world.step(TickInput::new(100)).expect("step");
    let poses = runtime.world.committed_pose_sources();
    let inputs: Vec<PoseInput> = poses
        .as_slice()
        .iter()
        .enumerate()
        .map(|(index, (_, source))| match *source {
            PoseSource::Lane { edge, progress } => {
                PoseInput::lane(PoseRecordId::new(index as u32), edge, progress)
            }
            PoseSource::Parking { space } => {
                PoseInput::parking(PoseRecordId::new(index as u32), space)
            }
        })
        .collect();
    let mut batch = CanonicalPoseBatch::new();
    runtime
        .session
        .extract_pose_batch(FramePlacementToken::new(1), &inputs, &mut batch)
        .expect("extract");
    let Some(record) = batch.records().first() else {
        return;
    };
    let Some(entity) = runtime.proxy else {
        return;
    };
    if let Ok(mut transform) = transforms.get_mut(entity) {
        let position = record.pose().position();
        *transform = Transform::from_xyz(position.x(), position.y(), position.z());
    }
}

#[test]
fn headless_app_steps_runtime_and_moves_proxy_transform() {
    let revision = revision();
    let mut world = TrafficWorld::install(Arc::clone(&revision), WorldConfig::new(8, 4, 1, 100))
        .expect("install");
    spawn_two_vehicles(&mut world);
    let session = SpatialSession::bind(revision)
        .expect("bind")
        .expect("session");
    let mut app = App::new();
    app.add_plugins((TimePlugin, TransformPlugin));
    app.insert_resource(MinRuntime {
        world,
        session,
        proxy: None,
    });
    app.add_systems(bevy_app::Startup, setup_proxy);
    app.add_systems(bevy_app::Update, step_runtime);
    app.update();
    let before = {
        let entity = app.world().resource::<MinRuntime>().proxy.expect("proxy");
        *app.world().get::<Transform>(entity).expect("transform")
    };
    for _ in 0..16 {
        app.update();
    }
    let after = {
        let entity = app.world().resource::<MinRuntime>().proxy.expect("proxy");
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
