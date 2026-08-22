//! 最小 Bevy 示例：`TrafficWorld` + 共享根 Spatial session 驱动代理位移。
//!
//! GUI 不进 CI。CI 通过 `tests/runtime_min_smoke.rs` 跑无窗口 App。

use std::{error::Error, sync::Arc};

use bevy::prelude::*;
use laneflow_format::{FormatLimits, check_canonical_network_input_v1};
use laneflow_runtime::{TickInput, TrafficWorld, VehicleSpawnInput, WorldConfig};
use laneflow_spatial::SpatialSession;
use laneflow_static_contract::{StaticRouteOrdinal, VehicleProfileOrdinal};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};

const FULL_SPATIAL: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable-v1/lfca-v1-full-spatial/expected.lfca"
);

fn main() -> Result<(), Box<dyn Error>> {
    let input = check_canonical_network_input_v1(FULL_SPATIAL, FormatLimits::V1_HARD)
        .map_err(|error| format!("{error:?}"))?;
    let revision = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .map_err(|error| format!("{error:?}"))?;
    let mut world = TrafficWorld::install(Arc::clone(&revision), WorldConfig::new(8, 4, 1, 100))?;
    let route = world.static_route(StaticRouteOrdinal::from_raw(0))?;
    world.spawn_vehicle(VehicleSpawnInput::new(
        VehicleProfileOrdinal::from_raw(0),
        route,
        0,
        1.0,
        0.0,
    ))?;
    let _session = SpatialSession::bind(revision)
        .map_err(|error| format!("{error:?}"))?
        .ok_or("missing spatial session")?;
    world.step(TickInput::new(100))?;

    App::new()
        .add_plugins(DefaultPlugins)
        .add_systems(Update, || {})
        .run();
    Ok(())
}
