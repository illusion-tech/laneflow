//! #696：同输入的生产命令测量；建世界、删除高水位车辆、断言与摘要均在计时外。
mod fixture;

use laneflow_runtime::{ParkingError, deterministic_state_digest};
use laneflow_static_network::SharedNetworkRevision;
use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

fn prepare(
    revision: &Arc<SharedNetworkRevision>,
    case: fixture::Case,
    shape: &str,
) -> fixture::Fixture {
    match shape {
        "normal" => fixture::fixture(revision, case),
        "capacity" => fixture::fixture_with_capacity(revision, case, 65_536),
        "high_water" => {
            assert_eq!(case.parked, fixture::EXITS);
            let mut f = fixture::fixture(
                revision,
                fixture::Case {
                    parked: 4_096,
                    ..case
                },
            );
            let removed = f.world.live_vehicles()[fixture::EXITS..4_096].to_vec();
            for vehicle in removed {
                f.world.despawn_vehicle(vehicle).unwrap();
            }
            f
        }
        _ => panic!("unknown storage shape"),
    }
}

fn run_case(
    revision: &Arc<SharedNetworkRevision>,
    case: fixture::Case,
    shape: &str,
    samples: usize,
    warmup: usize,
) {
    let mut calls = Vec::new();
    let mut batches = Vec::new();
    let mut reference = None;
    for sample in 0..samples + warmup {
        let mut f = prepare(revision, case, shape);
        let before = f.world.capture_snapshot().unwrap();
        let mut sum = 0;
        let mut successes = 0;
        for index in case.indices() {
            let start = Instant::now();
            let result = black_box(f.world.leave_parking(f.vehicles[index], f.targets[index]));
            let ns = start.elapsed().as_nanos();
            sum += ns;
            if sample >= warmup {
                calls.push((case.succeeds(index), ns));
            }
            if case.succeeds(index) {
                result.unwrap();
                successes += 1;
            } else {
                assert_eq!(
                    result,
                    Err(ParkingError::LeaveUnsafeFollower {
                        follower: f.followers[index]
                    })
                );
            }
        }
        if sample >= warmup {
            batches.push(sum);
        }
        let after = f.world.capture_snapshot().unwrap();
        assert_eq!(after.command_cursor(), before.command_cursor() + successes);
        assert_eq!(f.world.live_vehicles().len(), case.active + case.parked);
        if successes == 0 {
            assert_eq!(after, before);
        }
        let digest = deterministic_state_digest(&after).unwrap();
        if let Some(expected) = reference {
            assert_eq!(digest, expected);
        } else {
            reference = Some(digest);
        }
    }
    println!(
        "active-order case={case:?} shape={shape} samples={samples} warmup={warmup} digest={:x} batch_ns={batches:?} commands_ns={calls:?}",
        reference.unwrap()
    );
}

#[test]
fn active_order_evidence_smoke() {
    let revision = fixture::revision();
    let case = fixture::Case {
        active: 128,
        parked: 64,
        commands: 2,
        success_percent: 50,
        order: 0,
    };
    for shape in ["normal", "capacity", "high_water"] {
        run_case(&revision, case, shape, 1, 0);
    }
}

#[test]
#[ignore = "manual #696 release A/B, includes cold first success and later successful calls"]
fn active_order_release_matrix() {
    let revision = fixture::revision();
    for case in fixture::cases() {
        run_case(&revision, case, "normal", 16, 2);
    }
    for shape in ["capacity", "high_water"] {
        for active in [128, 1_024] {
            for success_percent in [0, 100] {
                run_case(
                    &revision,
                    fixture::Case {
                        active,
                        parked: 64,
                        commands: 64,
                        success_percent,
                        order: 0,
                    },
                    shape,
                    16,
                    2,
                );
            }
        }
    }
}
