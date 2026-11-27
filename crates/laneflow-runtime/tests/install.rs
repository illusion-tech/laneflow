#[path = "support/policy.rs"]
mod test_policy;

use std::num::NonZeroU32;
use std::sync::Arc;

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{
    CommittedNetworkSource, CutoverError, ExecutionConfig, ExecutionInitError, InstallError,
    PublishedLfcaReference, RouteError, RouteRegisterInput, SnapshotRestoreLimits, StepError,
    TickInput, TrafficWorld, WorldConfig, WorldConfigError, WorldConfigInput, WorldGeneration,
    WorldId, WorldInstallInput, encode_lfrs, restore_lfrs,
};
use laneflow_static_contract::{EntityKind, LaneEdgeOrdinal, NetworkRevisionId, Sha256Digest};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};

fn install_fixture(
    revision: Arc<SharedNetworkRevision>,
    config: WorldConfig,
) -> Result<TrafficWorld, InstallError> {
    TrafficWorld::install(install_input(
        revision,
        config,
        ExecutionConfig::new(NonZeroU32::MIN),
    ))
}

fn install_input(
    revision: Arc<SharedNetworkRevision>,
    config: WorldConfig,
    execution: ExecutionConfig,
) -> WorldInstallInput {
    let origin = *revision.canonical_origin();
    let policy_selection = test_policy::selection(&revision);
    WorldInstallInput {
        revision,
        config,
        execution,
        source: CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "fixture://in-process",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .unwrap(),
        },
        world_id: WorldId::new(0),
        policy_selection,
    }
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

const fn config_input(fixed_delta_time_ms: u64) -> WorldConfigInput {
    WorldConfigInput {
        vehicle_capacity: 8,
        route_capacity: 4,
        route_edge_occurrence_capacity: 512,
        route_conflict_occurrence_capacity: 1_024,
        fixed_delta_time_ms,
    }
}

fn config(delta_ms: u64) -> WorldConfig {
    WorldConfig::try_new(config_input(delta_ms)).expect("valid traffic configuration")
}

#[test]
fn named_install_preserves_distinct_fields_and_revision() {
    const CHECKED: Result<WorldConfig, WorldConfigError> = WorldConfig::try_new(config_input(100));
    let revision = revision();
    let input = install_input(
        Arc::clone(&revision),
        CHECKED.unwrap(),
        ExecutionConfig::new(NonZeroU32::MIN),
    );
    let summary = format!("{input:?}");
    assert!(summary.contains(&format!("{:?}", revision.network_revision())));
    assert!(summary.contains("Published"));
    assert!(!summary.contains("fixture://in-process"));
    let world = TrafficWorld::install(input).expect("install");
    let result = world.config();
    let actual = WorldConfigInput {
        vehicle_capacity: result.vehicle_capacity(),
        route_capacity: result.route_capacity(),
        route_edge_occurrence_capacity: result.route_edge_occurrence_capacity(),
        route_conflict_occurrence_capacity: result.route_conflict_occurrence_capacity(),
        fixed_delta_time_ms: result.fixed_delta_time_ms(),
    };
    assert_eq!(WorldConfig::try_new(actual), Ok(result));
    assert_eq!(actual, config_input(100));
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
    assert!(
        world
            .committed_pose_sources()
            .collect::<Vec<_>>()
            .is_empty()
    );
}

#[test]
fn config_rejects_invalid_step_before_install() {
    for dt in [0, 3, 1_001] {
        match WorldConfig::try_new(config_input(dt)).expect_err("invalid step") {
            WorldConfigError::DeltaOutOfRange { actual, min, max } => {
                assert_eq!((actual, min, max), (dt, 4, 1_000))
            }
            other => panic!("unexpected configuration error {other:?}"),
        }
    }
}

#[test]
fn zero_capacity_world_preserves_full_width_identity_on_restore() {
    let revision = revision();
    let config = WorldConfig::try_new(WorldConfigInput {
        vehicle_capacity: 0,
        route_capacity: 0,
        route_edge_occurrence_capacity: 0,
        route_conflict_occurrence_capacity: 0,
        fixed_delta_time_ms: 100,
    })
    .unwrap();
    let execution = ExecutionConfig::new(NonZeroU32::MIN);
    for raw in [0, u64::MAX] {
        let id = WorldId::new(raw);
        let world = TrafficWorld::install(WorldInstallInput {
            world_id: id,
            ..install_input(Arc::clone(&revision), config, execution)
        })
        .unwrap();
        let source = world.committed_source().clone();
        let snapshot = world.capture_snapshot().unwrap();
        assert_eq!(world.world_binding().world_id(), id);
        assert_eq!(snapshot.world_id(), id);
        let bytes = encode_lfrs(&snapshot);
        drop(world);
        let restored = restore_lfrs(
            &bytes,
            Arc::clone(&revision),
            source,
            config,
            execution,
            SnapshotRestoreLimits::new(16 * 1024 * 1024, 4096),
        )
        .unwrap();
        assert_eq!(restored.world().world_id().get(), raw);
        assert_eq!(
            restored.world().world_generation(),
            WorldGeneration::INITIAL
        );
        assert_eq!(
            encode_lfrs(&restored.world().capture_snapshot().unwrap()),
            bytes
        );
        let error = CutoverError::TransactionWorldMismatch { expected_world: id };
        assert_eq!(
            error.to_string(),
            format!("切换事务与传入世界不匹配（事务属世界 {raw}）")
        );
    }
}

#[test]
fn execution_config_accepts_workers_one_through_sixteen_and_rejects_higher() {
    assert_eq!(NonZeroU32::new(0), None);
    let revision = revision();
    for workers in [1, 2, 16] {
        let execution = ExecutionConfig::new(NonZeroU32::new(workers).unwrap());
        let world =
            TrafficWorld::install(install_input(Arc::clone(&revision), config(100), execution))
                .expect("supported worker count installs");
        assert_eq!(world.execution_config(), execution);
        assert_eq!(
            world.execution_config().worker_count(),
            NonZeroU32::new(workers).unwrap()
        );
    }
    for requested in [17, u32::MAX] {
        let execution = ExecutionConfig::new(NonZeroU32::new(requested).unwrap());
        assert_eq!(
            TrafficWorld::install(install_input(Arc::clone(&revision), config(100), execution))
                .map(|_| ())
                .unwrap_err(),
            InstallError::ExecutionInit(ExecutionInitError::UnsupportedWorkerCount {
                requested,
                max_supported: 16,
            })
        );
    }
}

#[test]
fn source_mismatch_precedes_signal_error_and_unsupported_execution() {
    let revision = revision();
    let installed_revision = revision.network_revision();
    let source_revision = NetworkRevisionId::from_digest(Sha256Digest::from_bytes([1; 32]));
    assert_ne!(source_revision, installed_revision);
    let execution = ExecutionConfig::new(NonZeroU32::new(17).unwrap());
    let input = install_input(Arc::clone(&revision), config(16), execution);
    let origin = revision.canonical_origin();
    let mismatched = CommittedNetworkSource::Published {
        reference: PublishedLfcaReference::new(
            "fixture://wrong-revision",
            origin.canonical_artifact_digest(),
            origin.canonical_artifact_byte_length(),
            source_revision,
        )
        .unwrap(),
    };
    assert_eq!(
        TrafficWorld::install(WorldInstallInput {
            source: mismatched,
            ..input
        })
        .err(),
        Some(InstallError::SourceRevisionMismatch {
            source_revision,
            installed_revision
        })
    );
    assert_eq!(
        TrafficWorld::install(install_input(Arc::clone(&revision), config(16), execution)).err(),
        Some(InstallError::PhaseNotMultipleOfTick)
    );
    assert_eq!(Arc::strong_count(&revision), 1);
    install_fixture(revision, config(100)).expect("retry with valid inputs");
}

#[test]
fn install_accepts_finest_and_coarsest_tick() {
    let revision = revision();
    install_fixture(Arc::clone(&revision), config(4)).expect("dt=4");
    install_fixture(revision, config(1_000)).expect("dt=1000");
}

#[test]
fn install_rejects_phase_not_multiple_of_tick() {
    let revision = revision();
    assert_eq!(
        install_fixture(Arc::clone(&revision), config(16))
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
    let mut world = install_fixture(revision(), config(100)).expect("install");
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
    let mut world = install_fixture(world_revision, config(100)).expect("install");
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
    let mut world = install_fixture(revision(), config(100)).expect("install");
    let outcome = world.step(TickInput::new(100)).expect("step");
    assert_eq!(outcome.tick_index(), 1);
    assert_eq!(outcome.time_ms(), 100);
    assert_eq!(world.tick_index(), 1);
    assert_eq!(world.time_ms(), 100);
}
