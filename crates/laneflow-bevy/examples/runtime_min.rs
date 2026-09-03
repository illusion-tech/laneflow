//! 最小 Bevy 示例：`LaneFlowPlugin` + `LaneFlowSession` 驱动车辆表现位移。
//!
//! GUI 不进 CI；`tests/runtime_min_smoke.rs` 复用初始化路径并运行无窗口 App。
#[path = "support/runtime_min_scene.rs"]
mod runtime_min_scene;

use std::error::Error;

use bevy::prelude::*;
use laneflow_bevy::{LaneFlowPlugin, LaneFlowSession, pose_input};
use laneflow_spatial::{CanonicalPoseBatch, FramePlacementToken, PoseRecordId};

fn main() -> Result<(), Box<dyn Error>> {
    let session = runtime_min_scene::session()?;

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
