#[path = "runtime_min_scene.rs"]
mod runtime_min_scene;

use std::time::Duration;

use bevy_app::App;
use bevy_ecs::{
    entity::Entity,
    resource::Resource,
    system::{Commands, Query, Res, ResMut},
};
use bevy_time::{TimePlugin, TimeUpdateStrategy};
use bevy_transform::{TransformPlugin, components::Transform};
use laneflow_bevy::{LaneFlowPlugin, LaneFlowSession, pose_input};
use laneflow_spatial::{CanonicalPoseBatch, FramePlacementToken, PoseRecordId};

#[derive(Resource)]
struct Proxy(Entity);

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
    let session = runtime_min_scene::session().expect("native example initialization");
    assert!(matches!(
        session.world().policy_selection(),
        laneflow_runtime::WorldPolicySelection::Pinned(_)
    ));
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
