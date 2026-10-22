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
use laneflow_bevy::{LaneFlowCommittedPoseBatch, LaneFlowPlugin, LaneFlowSession};
use laneflow_spatial::FramePlacementToken;

/// 跨帧复用的提取缓冲（adapter-api §6 稳定容量合同）。
#[derive(Resource, Default)]
struct PoseBuffer(LaneFlowCommittedPoseBatch);

#[derive(Resource)]
struct Proxy(Entity);

fn setup_proxy(mut commands: Commands) {
    let entity = commands.spawn(Transform::IDENTITY).id();
    commands.insert_resource(Proxy(entity));
}

fn sync_proxy(
    mut session: ResMut<LaneFlowSession>,
    mut poses: ResMut<PoseBuffer>,
    proxy: Option<Res<Proxy>>,
    mut transforms: Query<&mut Transform>,
) {
    session
        .extract_committed_pose_batch(FramePlacementToken::new(1), &mut poses.0)
        .expect("extract");
    let Some(record) = poses.0.batch().records().first() else {
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
    app.init_resource::<PoseBuffer>();
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
