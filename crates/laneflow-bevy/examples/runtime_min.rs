//! 最小 Bevy 示例：`LaneFlowPlugin` + `LaneFlowSession` 驱动车辆表现位移。
//!
//! GUI 不进 CI；`support/runtime_min_smoke.rs` 测试目标复用初始化路径并运行无窗口 App。
#[path = "support/runtime_min_scene.rs"]
mod runtime_min_scene;

use std::error::Error;

use bevy::prelude::*;
use laneflow_bevy::{LaneFlowCommittedPoseBatch, LaneFlowPlugin, LaneFlowSession};
use laneflow_spatial::FramePlacementToken;

/// 跨帧复用的提取缓冲（adapter-api §6 稳定容量合同）。
#[derive(Resource, Default)]
struct PoseBuffer(LaneFlowCommittedPoseBatch);

fn main() -> Result<(), Box<dyn Error>> {
    let session = runtime_min_scene::session()?;

    App::new()
        .add_plugins((DefaultPlugins, LaneFlowPlugin))
        .insert_resource(session)
        .init_resource::<PoseBuffer>()
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
    mut poses: ResMut<PoseBuffer>,
    proxy: Option<Res<Proxy>>,
    mut transforms: Query<&mut Transform>,
) {
    let Some(proxy) = proxy else {
        return;
    };
    if session
        .extract_committed_pose_batch(FramePlacementToken::new(1), &mut poses.0)
        .is_err()
    {
        return;
    }
    let Some(record) = poses.0.batch().records().first() else {
        return;
    };
    if let Ok(mut transform) = transforms.get_mut(proxy.0) {
        let position = record.pose().position();
        *transform = Transform::from_xyz(position.x(), position.y(), position.z());
    }
}
