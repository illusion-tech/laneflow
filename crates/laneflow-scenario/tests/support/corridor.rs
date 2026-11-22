//! 受保护转向走廊的真实世界安装（#1496、#1500 共用）：检入 LFCA 与 catalog、
//! 规范人口控制器与 16 ms 步长。

use std::sync::Arc;

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::TrafficWorld;
use laneflow_runtime::WorldConfig;
use laneflow_scenario::signalized_corridor::{
    CorridorCatalog, CorridorPopulationConfig, CorridorPopulationController,
    CorridorPopulationPrepare, PASSENGER_CAR_PROFILE_KEY, bind,
};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};

pub const CORRIDOR_LFCA: &[u8] =
    include_bytes!("../../../../examples/data/v0.2-signalized-corridor.lfca");
pub const CORRIDOR_CATALOG: &str =
    include_str!("../../../../examples/data/v0.2-signalized-corridor.catalog.toml");
pub const TICK_MS: u64 = 16;

fn install(revision: Arc<SharedNetworkRevision>, target: usize) -> TrafficWorld {
    let selection = laneflow_runtime::WorldPolicySelection::Pinned(laneflow_runtime::PolicyPin {
        policy: laneflow_static_contract::RightOfWayPolicySetId::from_untyped(
            laneflow_compiler::derive_canonical_stable_id_v1(
                laneflow_static_contract::EntityKind::RightOfWayPolicySet,
                "laneflow/signalized-corridor",
                "protected-entry",
            )
            .expect("policy identity"),
        ),
    });
    let origin = *revision.canonical_origin();
    TrafficWorld::install(
        Arc::clone(&revision),
        WorldConfig::new(
            u32::try_from(target).expect("fits"),
            28,
            1_024,
            1_024,
            TICK_MS,
        ),
        laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        laneflow_runtime::CommittedNetworkSource::Published {
            reference: laneflow_runtime::PublishedLfcaReference::new(
                "fixture://in-process",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("reference"),
        },
        0,
        selection,
    )
    .expect("install")
}

pub fn corridor(target: usize, seed: u64) -> (TrafficWorld, CorridorPopulationController) {
    let input = check_canonical_network_input(CORRIDOR_LFCA, FormatLimits::HARD).expect("LFCA");
    let revision = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .expect("revision");
    let catalog: CorridorCatalog = toml::from_str(CORRIDOR_CATALOG).expect("catalog");
    let bound = bind(&catalog, revision.as_ref()).expect("bind");
    let profile = *bound
        .profiles
        .get(PASSENGER_CAR_PROFILE_KEY)
        .expect("passenger-car");
    let config = CorridorPopulationConfig::try_new(target, seed).expect("config");
    let mut prepared =
        CorridorPopulationPrepare::prepare(config, bound, revision.as_ref(), profile)
            .expect("prepare");
    let mut world = install(Arc::clone(&revision), target);
    let (vehicles, routes) = prepared
        .spawn_initial_vehicles(&mut world)
        .expect("initial population");
    let controller = prepared.bind(&mut world, &vehicles, &routes).expect("bind");
    (world, controller)
}
