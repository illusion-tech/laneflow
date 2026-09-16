//! #678：正式生产库命令路径；测试计数和结构原型另在单元构建运行。
mod fixture;

use laneflow_runtime::{ParkingError, deterministic_state_digest};
use std::hint::black_box;
use std::time::Instant;

fn run_case(
    revision: &std::sync::Arc<laneflow_static_network::SharedNetworkRevision>,
    case: fixture::Case,
    samples: usize,
    warmup: usize,
) {
    let mut times = Vec::new();
    let mut batch_sums = Vec::new();
    let mut reference = None;
    for round in 0..samples + warmup {
        let mut f = fixture::fixture(revision, case);
        let before = f.world.capture_snapshot().unwrap();
        let mut sum = 0;
        let mut successes = 0;
        for i in case.indices() {
            let start = Instant::now();
            let result = black_box(f.world.leave_parking(f.vehicles[i], f.targets[i]));
            let ns = start.elapsed().as_nanos();
            sum += ns;
            if round >= warmup {
                times.push((case.succeeds(i), ns));
            }
            if case.succeeds(i) {
                result.unwrap();
                successes += 1;
            } else {
                assert_eq!(
                    result,
                    Err(ParkingError::LeaveUnsafeFollower {
                        follower: f.followers[i]
                    })
                );
            }
        }
        if round >= warmup {
            batch_sums.push(sum);
        }
        let after = f.world.capture_snapshot().unwrap();
        if successes == 0 {
            assert_eq!(before, after);
        }
        let digest = deterministic_state_digest(&after).unwrap();
        if let Some(expected) = reference {
            assert_eq!(expected, digest);
        }
        reference = Some(digest);
        assert_eq!(f.world.live_vehicles().len(), case.active + case.parked);
    }
    println!(
        "parking-command case={case:?} samples={samples} warmup={warmup} digest={:x} batch_ns={batch_sums:?} commands_ns={times:?}",
        reference.unwrap()
    );
}

#[test]
fn parking_command_matrix_smoke() {
    let revision = fixture::revision();
    for success_percent in [0, 50, 100] {
        run_case(
            &revision,
            fixture::Case {
                active: 128,
                parked: 64,
                commands: 4,
                success_percent,
                order: 0,
            },
            1,
            0,
        );
    }
}

#[test]
#[ignore = "manual release command latency; no test-only library counters"]
fn parking_command_release_matrix() {
    let revision = fixture::revision();
    for case in fixture::cases() {
        run_case(&revision, case, 16, 2);
    }
}
