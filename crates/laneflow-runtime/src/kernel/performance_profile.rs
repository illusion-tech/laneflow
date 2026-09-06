//! #583 测试构建中的批次阶段墙钟；无生产 feature、API 或状态字段。

use std::cell::Cell;
use std::time::Instant;

use crate as runtime_types;
#[path = "tests/performance_profile/runtime_profile.rs"]
mod support;

const STAGE_COUNT: usize = 11;

#[derive(Clone, Copy)]
pub(crate) enum Stage {
    WholeStep,
    Preflight,
    Occupancy,
    WaitingPrepare,
    ConflictPrepare,
    MotionLoop,
    WaitingFinalize,
    Signals,
    ConflictFinalize,
    WaitingOutputs,
    Commit,
}

const NAMES: [&str; STAGE_COUNT] = [
    "whole_step",
    "preflight",
    "occupancy",
    "waiting_prepare",
    "conflict_prepare",
    "motion_loop",
    "waiting_finalize",
    "signals",
    "conflict_finalize",
    "waiting_outputs",
    "commit",
];

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    static NANOS: Cell<[u128; STAGE_COUNT]> = const { Cell::new([0; STAGE_COUNT]) };
    static CALLS: Cell<[u64; STAGE_COUNT]> = const { Cell::new([0; STAGE_COUNT]) };
}

pub(crate) struct Span(Option<(Stage, Instant)>);

pub(crate) fn begin(stage: Stage) -> Span {
    Span(ENABLED.with(|enabled| enabled.get().then(|| (stage, Instant::now()))))
}

impl Drop for Span {
    fn drop(&mut self) {
        if let Some((stage, started)) = self.0 {
            let elapsed = started.elapsed().as_nanos();
            NANOS.with(|values| {
                let mut next = values.get();
                next[stage as usize] += elapsed;
                values.set(next);
            });
            CALLS.with(|values| {
                let mut next = values.get();
                next[stage as usize] += 1;
                values.set(next);
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "manual release instrumented batch-stage and retained-memory profile"]
    fn runtime_profile_stages() {
        let fixtures = support::Fixtures::new();
        for scene in support::CASES {
            for round in 0..3 {
                let mut harness = fixtures.world(scene);
                harness.validate();
                NANOS.set([0; STAGE_COUNT]);
                CALLS.set([0; STAGE_COUNT]);
                ENABLED.set(true);
                for _ in 0..harness.steps {
                    let _whole = begin(Stage::WholeStep);
                    harness.step();
                }
                ENABLED.set(false);
                harness.validate();
                let nanos = NANOS.get();
                let calls = CALLS.get();
                assert!(calls.iter().all(|count| *count == harness.steps as u64));
                assert!(nanos.iter().skip(1).sum::<u128>() <= nanos[0]);
                let memory = harness.world.retained_memory();
                println!(
                    "profile-memory scene={} round={round} source_world_owned={} shared_root={} binding={} committed={} derived={} workspace={} administrative={} journal_bytes={} digest={}",
                    scene.name(),
                    memory.world_owned_bytes(),
                    memory.shared_network,
                    memory.partitions[0],
                    memory.partitions[1],
                    memory.partitions[2],
                    memory.partitions[3],
                    memory.partitions[4],
                    harness.journal_bytes(),
                    harness.digest(),
                );
                for index in 0..STAGE_COUNT {
                    println!(
                        "profile-stage scene={} round={round} stage={} calls={} ns={}",
                        scene.name(),
                        NAMES[index],
                        calls[index],
                        nanos[index],
                    );
                }
            }
        }
    }
}
