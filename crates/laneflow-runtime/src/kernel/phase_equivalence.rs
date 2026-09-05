//! 相同文件在 e011745e94986a17049c66e730d54ac9fccc59f9 与当前实现运行。
//! 只比较公开状态/批次及既有日志记录；不读取分区、容量或私有 grant serial。

use super::{STEP_FAILPOINT, StepFailpoint};
use crate::{StepError, TickInput, TrafficWorld};
use sha2::{Digest, Sha256};
use std::sync::Arc;

fn checkpoint(world: &TrafficWorld) -> String {
    let snapshot = world.capture_snapshot().unwrap();
    let vehicles: Vec<_> = world
        .live_vehicles()
        .iter()
        .map(|id| world.vehicle(*id))
        .collect();
    format!(
        "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
        crate::deterministic_state_digest(&snapshot).unwrap(),
        world.live_vehicles(),
        vehicles,
        world.latest_waiting_decisions(),
        world.latest_conflict_decisions(),
        world.latest_transition_events(),
        world.committed_signal_groups(),
        world.waiting_zone_members(),
        world.migration_journal_stats(),
        (
            world.observation_state_sequence(),
            world.world_generation(),
            world.command_cursor(),
            world.event_cursor()
        )
    )
}

fn journal(world: &TrafficWorld) -> String {
    format!(
        "{:?}",
        world
            .migration_journal()
            .filter(|log| !log.overflowed())
            .map(|log| log.records_from(0).collect::<Vec<_>>())
    )
}

fn trace(mut world: TrafficWorld, ticks: usize, retry: bool, journal_bound: Option<u64>) -> String {
    if let Some(bound) = journal_bound {
        world.arm_migration_journal(bound).unwrap();
    }
    let mut digest = Sha256::new();
    digest.update(checkpoint(&world).as_bytes());
    let input = TickInput::new(world.config().fixed_delta_time_ms());
    let mut event_count = 0;
    for tick in 0..ticks {
        if retry && tick < 2 {
            let before = checkpoint(&world);
            let journal_before = journal(&world);
            let point = if tick == 0 {
                StepFailpoint::AfterGrants
            } else {
                StepFailpoint::AfterTransitions
            };
            STEP_FAILPOINT.with(|slot| slot.set(Some(point)));
            assert_eq!(
                world.step(input),
                Err(StepError::ParkingObservationAllocFailed)
            );
            assert_eq!(
                checkpoint(&world),
                before,
                "failed staging preserves all published state"
            );
            assert_eq!(
                journal(&world),
                journal_before,
                "failed staging never appends a journal frame"
            );
        }
        let outcome = world.step(input).unwrap();
        event_count += world.latest_transition_events().len();
        digest.update(
            format!(
                "{outcome:?}|{}
",
                checkpoint(&world)
            )
            .as_bytes(),
        );
    }
    digest.update(journal(&world).as_bytes());
    // 恢复不重建历史 latest batches；按快照局部身份比较后续语义。
    let captured = world.capture_snapshot().unwrap();
    let mut restored = crate::restore_lfrs(
        &crate::encode_lfrs(&captured),
        world.revision(),
        world.committed_source().clone(),
        world.config(),
        crate::SnapshotRestoreLimits::new(16 * 1_024 * 1_024, 4_096),
    )
    .unwrap()
    .into_world();
    assert_eq!(
        crate::deterministic_state_digest(&restored.capture_snapshot().unwrap()).unwrap(),
        crate::deterministic_state_digest(&captured).unwrap()
    );
    world.step(input).unwrap();
    restored.step(input).unwrap();
    let continued = crate::deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap();
    assert_eq!(
        crate::deterministic_state_digest(&restored.capture_snapshot().unwrap()).unwrap(),
        continued
    );
    digest.update(format!("{continued:?}").as_bytes());
    eprintln!(
        "phase-trace-cost vehicles={} ticks={ticks} retry={retry} journal_bound={journal_bound:?} conflict_retained_bytes={}",
        world.live_vehicles().len(),
        world.conflict_retained_logical_bytes()
    );
    let hex: String = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{hex}:{event_count}")
}

fn signals_world() -> TrafficWorld {
    let input = laneflow_format::check_canonical_network_input(
        include_bytes!(
            "../../../laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfca"
        ),
        laneflow_format::FormatLimits::HARD,
    )
    .unwrap();
    let revision = laneflow_static_network::build_shared_network_revision(
        input,
        laneflow_static_network::SharedNetworkBuildOptions::new(
            laneflow_static_network::SpatialBuildOption::Omit,
            laneflow_static_network::SharedNetworkBuildLimits::new(
                64 * 1_024 * 1_024,
                16 * 1_024 * 1_024,
            ),
        ),
    )
    .unwrap();
    let origin = *revision.canonical_origin();
    TrafficWorld::install(
        Arc::clone(&revision),
        crate::WorldConfig::new(4, 4, 1_024, 1_024, 1, 100),
        crate::CommittedNetworkSource::Published {
            reference: crate::PublishedLfcaReference::new(
                "fixture://phase-signals",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .unwrap(),
        },
        581,
        crate::test_policy::selection(&revision),
    )
    .unwrap()
}

#[test]
fn exact_baseline_trace_and_retry_match() {
    let revision = crate::admin::cutover_migration::tests::conflict_scale_revision();
    let mut actual = Vec::new();
    for (name, count, ticks, bound) in [
        ("waiting", 2, 160, None),
        ("waiting-journal", 2, 160, Some(1_024 * 1_024)),
        ("waiting-overflow", 2, 12, Some(32)),
    ] {
        let baseline = trace(
            crate::kernel::waiting::tests::multi_gate_world(count),
            ticks,
            false,
            bound,
        );
        assert_eq!(
            trace(
                crate::kernel::waiting::tests::multi_gate_world(count),
                ticks,
                true,
                bound
            ),
            baseline
        );
        actual.push(format!("{name}={baseline}"));
    }
    for (name, bound) in [
        ("conflict", None),
        ("conflict-journal", Some(1_024 * 1_024)),
    ] {
        let baseline = trace(
            crate::admin::cutover_migration::tests::conflict_scale_world(Arc::clone(&revision), 2),
            640,
            false,
            bound,
        );
        assert_eq!(
            trace(
                crate::admin::cutover_migration::tests::conflict_scale_world(
                    Arc::clone(&revision),
                    2
                ),
                640,
                true,
                bound
            ),
            baseline
        );
        actual.push(format!("{name}={baseline}"));
    }
    let signals = trace(signals_world(), 600, false, Some(1_024 * 1_024));
    assert_eq!(
        trace(signals_world(), 600, true, Some(1_024 * 1_024)),
        signals
    );
    actual.push(format!("signals-clock={signals}"));
    let actual = actual.join("\n");
    eprintln!("PHASE_BASELINE_BEGIN\n{actual}\nPHASE_BASELINE_END");
    // Filled from the fixed pre-refactor commit, never regenerated from the implementation under test.
    const EXPECTED: &str = include_str!("../../tests/fixtures/phase-protocol-e011745e.txt");
    assert_eq!(actual, EXPECTED.trim_end());
}

#[test]
fn input_and_preparation_errors_precede_staged_failure() {
    for waiting in [true, false] {
        let mut world = if waiting {
            crate::kernel::waiting::tests::multi_gate_world(2)
        } else {
            crate::admin::cutover_migration::tests::conflict_scale_world(
                crate::admin::cutover_migration::tests::conflict_scale_revision(),
                2,
            )
        };
        let before = checkpoint(&world);
        let delta = world.config().fixed_delta_time_ms();
        STEP_FAILPOINT.with(|slot| slot.set(Some(StepFailpoint::AfterGrants)));
        assert!(matches!(
            world.step(TickInput::new(delta + 1)),
            Err(StepError::DeltaMismatch { .. })
        ));
        crate::kernel::conflict::set_allocation_failpoint(Some(0));
        let result = world.step(TickInput::new(delta));
        crate::kernel::conflict::set_allocation_failpoint(None);
        STEP_FAILPOINT.with(|slot| slot.set(None));
        assert_eq!(result, Err(StepError::ConflictScratchAllocFailed));
        assert_eq!(checkpoint(&world), before);
        world.step(TickInput::new(delta)).unwrap();
    }
}
