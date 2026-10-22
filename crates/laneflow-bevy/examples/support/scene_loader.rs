//! example 与无窗口 smoke 共用的宿主装载路径：检入 LFCA → 共享路网修订 →
//! `TrafficWorld` → 配对 `LaneFlowSession`。
//!
//! 只服务仓库 example；不是 `laneflow-bevy` 公开 API。

use std::{error::Error, num::NonZeroU32, sync::Arc};

use laneflow_bevy::{LaneFlowSession, LaneFlowSessionConfig};
use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{
    CommittedNetworkSource, ExecutionConfig, PublishedLfcaReference, TrafficWorld, WorldConfig,
    WorldPolicySelection,
};
use laneflow_spatial::SpatialSession;
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};

/// 检查 LFCA 并构建保留 Spatial component 的共享路网修订。
pub fn shared_revision(lfca: &[u8]) -> Result<Arc<SharedNetworkRevision>, Box<dyn Error>> {
    let input = check_canonical_network_input(lfca, FormatLimits::HARD)
        .map_err(|error| format!("{error:?}"))?;
    let revision = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .map_err(|error| format!("{error:?}"))?;
    Ok(revision)
}

/// 以检入制品作为已发布来源安装单线程世界；`scenario_key` 是宿主资产键。
pub fn install_world(
    revision: &Arc<SharedNetworkRevision>,
    scenario_key: &str,
    config: WorldConfig,
    policy_selection: WorldPolicySelection,
) -> Result<TrafficWorld, Box<dyn Error>> {
    let origin = revision.canonical_origin();
    let reference = PublishedLfcaReference::new(
        scenario_key,
        origin.canonical_artifact_digest(),
        origin.canonical_artifact_byte_length(),
        origin.network_revision(),
    )
    .map_err(|error| format!("{error:?}"))?;
    Ok(TrafficWorld::install(
        Arc::clone(revision),
        config,
        ExecutionConfig::new(NonZeroU32::MIN),
        CommittedNetworkSource::Published { reference },
        0,
        policy_selection,
    )?)
}

/// 用世界自身的共享路网修订绑定 Spatial，组成配对 Session。
pub fn paired_session(
    world: TrafficWorld,
    max_catch_up_steps: NonZeroU32,
) -> Result<LaneFlowSession, Box<dyn Error>> {
    let spatial = SpatialSession::bind(world.revision())
        .map_err(|error| format!("{error:?}"))?
        .ok_or("revision has no spatial component")?;
    Ok(LaneFlowSession::new(
        world,
        Some(spatial),
        LaneFlowSessionConfig::new(max_catch_up_steps),
    )?)
}
