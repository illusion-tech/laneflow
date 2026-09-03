#[path = "support/policy.rs"]
mod test_policy;

use std::sync::Arc;

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{
    InstallError, RouteError, RouteRegisterInput, StepError, TickInput, TrafficWorld, WorldConfig,
};
use laneflow_static_contract::{EntityKind, LaneEdgeOrdinal};
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
        std::sync::Arc::clone(&revision),
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
        test_policy::selection(&revision),
    )
}

const FULL_SPATIAL: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfca"
);

const BUILD_LIMITS: SharedNetworkBuildLimits =
    SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024);

fn revision() -> Arc<laneflow_static_network::SharedNetworkRevision> {
    let input = check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD)
        .expect("checked canonical network input");
    build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, BUILD_LIMITS),
    )
    .expect("shared network revision")
}

fn config(delta_ms: u64, workers: u32) -> WorldConfig {
    WorldConfig::new(8, 4, 1_024, 1_024, workers, delta_ms)
}

#[test]
fn install_full_spatial_retains_single_arc() {
    let revision = revision();
    let world = install_fixture(Arc::clone(&revision), config(100, 1)).expect("install");
    assert!(Arc::ptr_eq(&world.revision(), &revision));
    assert_eq!(world.tick_index(), 0);
    assert_eq!(world.time_ms(), 0);
    assert_eq!(world.config().route_edge_occurrence_capacity(), 1_024);
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
fn install_rejects_delta_out_of_range_and_non_one_worker() {
    let revision = revision();
    assert_eq!(
        install_fixture(Arc::clone(&revision), config(0, 1))
            .map(|_| ())
            .unwrap_err(),
        InstallError::DeltaOutOfRange {
            actual: 0,
            min: 4,
            max: 1_000,
        }
    );
    assert_eq!(
        install_fixture(Arc::clone(&revision), config(3, 1))
            .map(|_| ())
            .unwrap_err(),
        InstallError::DeltaOutOfRange {
            actual: 3,
            min: 4,
            max: 1_000,
        }
    );
    assert_eq!(
        install_fixture(Arc::clone(&revision), config(1_001, 1))
            .map(|_| ())
            .unwrap_err(),
        InstallError::DeltaOutOfRange {
            actual: 1_001,
            min: 4,
            max: 1_000,
        }
    );
    assert_eq!(
        install_fixture(Arc::clone(&revision), config(100, 2))
            .map(|_| ())
            .unwrap_err(),
        InstallError::WorkerCountNotOne
    );
}

#[test]
fn install_accepts_finest_and_coarsest_tick() {
    let revision = revision();
    install_fixture(Arc::clone(&revision), config(4, 1)).expect("dt=4");
    install_fixture(revision, config(1_000, 1)).expect("dt=1000");
}

#[test]
fn install_rejects_phase_not_multiple_of_tick() {
    let revision = revision();
    assert_eq!(
        install_fixture(Arc::clone(&revision), config(16, 1))
            .map(|_| ())
            .unwrap_err(),
        InstallError::PhaseNotMultipleOfTick
    );
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

#[test]
fn remove_route_rejects_stale_handle() {
    let mut world = install_fixture(revision(), config(100, 1)).expect("install");
    let route = world
        .register_route(RouteRegisterInput::new(vec![
            edge_for_length(&world, 10_000),
            edge_for_length(&world, 8_000),
            edge_for_length(&world, 12_000),
        ]))
        .expect("register");
    world.remove_route(route).expect("unused");
    assert_eq!(
        world.remove_route(route).unwrap_err(),
        RouteError::StaleHandle
    );
}

#[test]
fn step_rejects_delta_mismatch_without_advancing() {
    let world_revision = revision();
    let mut world = install_fixture(world_revision, config(100, 1)).expect("install");
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
    let mut world = install_fixture(revision(), config(100, 1)).expect("install");
    let outcome = world.step(TickInput::new(100)).expect("step");
    assert_eq!(outcome.tick_index(), 1);
    assert_eq!(outcome.time_ms(), 100);
    assert_eq!(world.tick_index(), 1);
    assert_eq!(world.time_ms(), 100);
}
