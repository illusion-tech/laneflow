use std::sync::Arc;

use laneflow_format::{FormatLimits, check_canonical_network_input_v1};
use laneflow_runtime::{
    InstallError, LookupError, StepError, TickInput, TrafficWorld, WorldConfig,
};
use laneflow_static_contract::{EntityKind, StaticRouteOrdinal};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};

const FULL_SPATIAL: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable-v1/lfca-v1-full-spatial/expected.lfca"
);

const BUILD_LIMITS: SharedNetworkBuildLimits =
    SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024);

fn revision() -> Arc<laneflow_static_network::SharedNetworkRevision> {
    let input = check_canonical_network_input_v1(FULL_SPATIAL, FormatLimits::V1_HARD)
        .expect("checked canonical network input");
    build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, BUILD_LIMITS),
    )
    .expect("shared network revision")
}

fn config(delta_ms: u64, workers: u32) -> WorldConfig {
    WorldConfig::new(8, 4, workers, delta_ms)
}

#[test]
fn install_full_spatial_retains_single_arc() {
    let revision = revision();
    let world = TrafficWorld::install(Arc::clone(&revision), config(100, 1)).expect("install");
    assert!(Arc::ptr_eq(&world.revision(), &revision));
    assert_eq!(world.tick_index(), 0);
    assert_eq!(world.time_ms(), 0);
    assert!(
        !world.committed_signal_groups().as_slice().is_empty()
            || revision
                .traffic()
                .entity_counts()
                .count(EntityKind::SignalGroup)
                == 0
    );
    assert!(world.committed_pose_sources().as_slice().is_empty());
}

#[test]
fn install_rejects_non_positive_delta_and_non_one_worker() {
    let revision = revision();
    assert_eq!(
        TrafficWorld::install(Arc::clone(&revision), config(0, 1))
            .map(|_| ())
            .unwrap_err(),
        InstallError::NonPositiveDelta
    );
    assert_eq!(
        TrafficWorld::install(Arc::clone(&revision), config(100, 2))
            .map(|_| ())
            .unwrap_err(),
        InstallError::WorkerCountNotOne
    );
}

#[test]
fn install_rejects_phase_shorter_than_tick() {
    let revision = revision();
    let err = TrafficWorld::install(revision, config(u64::MAX, 1))
        .map(|_| ())
        .unwrap_err();
    assert_eq!(err, InstallError::PhaseShorterThanTick);
}

#[test]
fn static_route_rejects_out_of_range() {
    let revision = revision();
    let count = revision
        .traffic()
        .entity_counts()
        .count(EntityKind::StaticRoute);
    let world = TrafficWorld::install(revision, config(100, 1)).expect("install");
    assert!(world.static_route(StaticRouteOrdinal::from_raw(0)).is_ok() || count == 0);
    assert_eq!(
        world
            .static_route(StaticRouteOrdinal::from_raw(count))
            .unwrap_err(),
        LookupError::UnknownStaticRoute
    );
}

#[test]
fn step_rejects_delta_mismatch_without_advancing() {
    let world_revision = revision();
    let mut world = TrafficWorld::install(world_revision, config(100, 1)).expect("install");
    let err = world.step(TickInput::new(50)).unwrap_err();
    assert_eq!(
        err,
        StepError::DeltaMismatch {
            expected_delta_time_ms: 100,
            actual_delta_time_ms: 50,
        }
    );
    assert_eq!(world.tick_index(), 0);
    assert_eq!(world.time_ms(), 0);
}

#[test]
fn step_advances_tick_and_time() {
    let mut world = TrafficWorld::install(revision(), config(100, 1)).expect("install");
    let outcome = world.step(TickInput::new(100)).expect("step");
    assert_eq!(outcome.tick_index(), 1);
    assert_eq!(outcome.time_ms(), 100);
    assert_eq!(world.tick_index(), 1);
    assert_eq!(world.time_ms(), 100);
}
