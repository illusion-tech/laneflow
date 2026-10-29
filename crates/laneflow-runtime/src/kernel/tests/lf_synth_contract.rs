//! #1148 仅验证小样本交通合同，不作为 LF-SYNTH 规模、表现或性能认证。
#[path = "lf_synth_contract/fixture.rs"]
mod fixture;
#[path = "lf_synth_contract/formal_retry.rs"]
mod formal_retry;

use crate::kernel::tick::{STEP_FAILPOINT, StepFailpoint};
use crate::*;
use fixture::*;
use laneflow_static_contract::{SignalAspect, VehicleProfileOrdinal};

#[test]
fn signal_quantization_and_old_input_rejection() {
    assert_eq!(durations(16), [30_000, 3_008, 1_008, 20_000, 3_008, 1_008]);
    assert_eq!(durations(33), [30_030, 3_003, 1_023, 20_031, 3_003, 1_023]);
    for (dt, expected) in [(16, 58_032), (33, 58_113)] {
        assert_eq!(cycle(dt), expected);
        assert!(matches!(
            install(revision(1, dt, false, true), dt),
            Err(InstallError::PhaseNotMultipleOfTick)
        ));
        let mut s = Sample::new(1, dt, Case::W4);
        let route = s.routes[3];
        assert_eq!(
            s.world
                .spawn_vehicle(VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    1,
                    EDGE_MM - 100,
                    13_900
                ))
                .unwrap_err(),
            SpawnError::StopConstraintUnsatisfiable
        );
        // 终点零速仍是 Active，下一拍自然提交 Completed；不绕过生产准入。
        let h = s
            .world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                1,
                EDGE_MM,
                0,
            ))
            .unwrap();
        assert_eq!(s.world.vehicle(h).unwrap().status(), VehicleStatus::Active);
        s.tick();
        assert_eq!(
            s.world.vehicle(h).unwrap().status(),
            VehicleStatus::Completed
        );
    }
}

#[test]
fn dense_moving_initialization_is_rejected_in_both_orders() {
    for dt in [16, 33] {
        for reverse in [false, true] {
            let mut s = Sample::new(1, dt, Case::W4);
            for h in s.handles.clone() {
                s.world.despawn_vehicle(h).unwrap();
            }
            let positions = if reverse {
                [26_500, 20_000]
            } else {
                [20_000, 26_500]
            };
            let first = s
                .world
                .spawn_vehicle(input(&s, 0, 0, positions[0], 1_000))
                .unwrap();
            let before = s.world.capture_snapshot().unwrap();
            let result = s.world.spawn_vehicle(input(&s, 0, 0, positions[1], 1_000));
            assert_eq!(
                result,
                Err(if reverse {
                    SpawnError::UnsafeLeader { leader: first }
                } else {
                    SpawnError::UnsafeFollower { follower: first }
                })
            );
            assert_eq!(s.world.capture_snapshot().unwrap(), before);
        }
    }
}

#[test]
fn w1_w2_population_and_parking_are_stable_for_full_window() {
    for case in [Case::W1, Case::W2] {
        for cells in [1, 2] {
            for dt in [16, 33] {
                let mut s = Sample::new(cells, dt, case);
                let cursor = s.world.command_cursor();
                let bindings: Vec<_> = s
                    .handles
                    .iter()
                    .map(|h| s.world.parking_binding(*h))
                    .collect();
                let (active, parked) = if case == Case::W1 { (75, 25) } else { (100, 0) };
                s.signals();
                for _ in 0..12 * cycle(dt) / dt {
                    assert!(s.tick().parking_arrivals().is_empty());
                    s.population(active, parked, 0);
                    assert_eq!(s.world.command_cursor(), cursor);
                    for (&handle, &binding) in s.handles.iter().zip(&bindings) {
                        assert_eq!(s.world.parking_binding(handle), binding);
                    }
                }
                println!(
                    "{case:?} cells={cells} dt_ms={dt} ticks={} population={active}/{parked}",
                    s.world.tick_index()
                );
            }
        }
    }
}

fn input(s: &Sample, rank: usize, edge: u32, progress: u32, speed: u32) -> VehicleSpawnInput {
    VehicleSpawnInput::new(
        VehicleProfileOrdinal::from_raw(0),
        s.routes[rank / 25],
        edge,
        progress,
        speed,
    )
}

fn terminal_spawn(s: &mut Sample, slot: usize) {
    for c in 0..s.cells {
        let rank = c * 100 + 75 + slot;
        s.world.despawn_vehicle(s.handles[rank]).unwrap();
        s.handles[rank] = s
            .world
            .spawn_vehicle(input(s, rank, 1, EDGE_MM, 0))
            .unwrap();
    }
}

fn replace(s: &mut Sample, slot: usize, progress: u32) {
    for c in 0..s.cells {
        let rank = c * 100 + 75 + slot;
        let old = s.handles[rank];
        assert_eq!(
            s.world.vehicle(old).unwrap().status(),
            VehicleStatus::Completed
        );
        s.handles[rank] = s
            .world
            .replace_completed_vehicle(old, input(s, rank, 0, progress, 13_900))
            .unwrap()
            .new;
        assert_ne!(s.handles[rank], old);
        assert!(s.world.vehicle(old).is_none());
    }
}

fn leave(s: &mut Sample, local_rank: usize) {
    for c in 0..s.cells {
        let rank = c * 100 + local_rank;
        s.world
            .leave_parking(
                s.handles[rank],
                LeaveParkingTarget::ExplicitSpace {
                    space: s.spaces[rank],
                    route: s.routes[rank / 25],
                    exit_route_occurrence: 1,
                },
            )
            .unwrap();
        assert_eq!(s.world.vehicle(s.handles[rank]).unwrap().speed_mm_s(), 0);
    }
}

fn equal(a: &Sample, b: &Sample) {
    assert_eq!(a.handles, b.handles);
    assert_eq!(
        deterministic_state_digest(&a.world.capture_snapshot().unwrap()).unwrap(),
        deterministic_state_digest(&b.world.capture_snapshot().unwrap()).unwrap()
    );
    assert_eq!(
        a.world.capture_snapshot().unwrap(),
        b.world.capture_snapshot().unwrap()
    );
    assert_eq!(
        a.world.latest_transition_events(),
        b.world.latest_transition_events()
    );
    assert_eq!(
        a.world.committed_signal_groups(),
        b.world.committed_signal_groups()
    );
}

fn paired_tick(a: &mut Sample, b: &mut Sample, fail: bool) -> StepOutcome {
    if fail {
        equal(a, b);
        let before = a.world.capture_snapshot().unwrap();
        let digest = deterministic_state_digest(&before).unwrap();
        let events = a.world.latest_transition_events().to_vec();
        let cursor = a.world.command_cursor();
        STEP_FAILPOINT.set(Some(StepFailpoint::AfterTransitions));
        assert_eq!(
            a.world.step(TickInput::new(u64::from(a.dt))),
            Err(StepError::ParkingObservationAllocFailed)
        );
        assert_eq!(a.world.capture_snapshot().unwrap(), before);
        assert_eq!(
            deterministic_state_digest(&a.world.capture_snapshot().unwrap()).unwrap(),
            digest
        );
        assert_eq!(a.world.latest_transition_events(), events);
        assert_eq!(a.world.command_cursor(), cursor);
    }
    let result = a.tick();
    assert_eq!(result, b.tick());
    assert_eq!(
        a.world.latest_transition_events(),
        b.world.latest_transition_events()
    );
    if fail {
        equal(a, b);
    }
    result
}

#[test]
fn w4_causal_boundaries_and_retry_match_fresh_replay() {
    for cells in [1, 2] {
        for dt in [16, 33] {
            let mut a = Sample::new(cells, dt, Case::W4);
            let mut b = Sample::new(cells, dt, Case::W4);
            let warmup = 4 * cycle(dt) / dt;
            for _ in 0..warmup - 1 {
                paired_tick(&mut a, &mut b, false);
            }
            for s in [&mut a, &mut b] {
                terminal_spawn(s, 12);
            }
            paired_tick(&mut a, &mut b, false);
            a.population(74, 25, 1);
            assert!(
                a.world
                    .committed_signal_groups()
                    .as_slice()
                    .iter()
                    .all(|(_, aspect)| *aspect == SignalAspect::Red)
            );
            for s in [&mut a, &mut b] {
                replace(s, 12, 356_000);
                leave(s, 62);
                s.population(76, 24, 0);
            }
            paired_tick(&mut a, &mut b, true);
            assert_eq!(
                a.world
                    .committed_signal_groups()
                    .as_slice()
                    .iter()
                    .filter(|(_, aspect)| *aspect == SignalAspect::Green)
                    .count(),
                cells * 2
            );
            for s in [&mut a, &mut b] {
                for c in 0..cells {
                    let rank = c * 100 + 61;
                    s.world.despawn_vehicle(s.handles[rank]).unwrap();
                    s.handles[rank] = s
                        .world
                        .spawn_vehicle(input(s, rank, 0, EDGE_MM - 100, 13_900))
                        .unwrap();
                }
            }
            paired_tick(&mut a, &mut b, false);
            for c in 0..cells {
                let handle = a.handles[c * 100 + 61];
                assert_eq!(a.world.vehicle(handle).unwrap().route_edge_index(), 1);
                assert_eq!(
                    a.world
                        .latest_transition_events()
                        .iter()
                        .filter(|event| event.vehicle() == handle
                            && matches!(event.kind(), TrafficTransitionKind::GateCrossed { .. }))
                        .count(),
                    1
                );
            }
            a.population(76, 24, 0);
            assert_eq!(a.world.tick_index(), u64::from(warmup + 2));
            equal(&a, &b);
            println!(
                "W4 cells={cells} dt_ms={dt} B0={} B1={} retry=fresh",
                warmup + 1,
                warmup + 2
            );
        }
    }
}

#[test]
fn w3_arrival_barrier_lifecycle_and_retry_match_fresh_replay() {
    for cells in [1, 2] {
        for dt in [16, 33] {
            let mut a = Sample::new(cells, dt, Case::W3);
            let mut b = Sample::new(cells, dt, Case::W3);
            for s in [&mut a, &mut b] {
                s.population(25, 75, 0);
                for c in 0..cells {
                    let rank = c * 100;
                    assert!(matches!(
                        s.world
                            .reserve_parking(
                                s.handles[rank],
                                ReserveParkingTarget::ExplicitSpace {
                                    space: s.spaces[rank],
                                    entry_route_occurrence: 1,
                                }
                            )
                            .unwrap(),
                        ParkingCommandOutcome::Committed(_)
                    ));
                }
            }
            let warmup = 4 * cycle(dt) / dt;
            for _ in 0..warmup {
                assert!(
                    paired_tick(&mut a, &mut b, false)
                        .parking_arrivals()
                        .is_empty()
                );
            }
            let mut arrived = vec![false; cells];
            let mut parked_tick = None;
            let mut left = false;
            for tick in 0..8 * cycle(dt) / dt {
                for s in [&mut a, &mut b] {
                    if tick == 0 {
                        terminal_spawn(s, 6);
                    }
                    if tick == 1 {
                        replace(s, 6, 188_000);
                    }
                    if arrived.iter().all(|x| *x) && parked_tick.is_none() {
                        for c in 0..cells {
                            let rank = c * 100;
                            assert!(matches!(
                                s.world
                                    .park_vehicle(
                                        s.handles[rank],
                                        ParkingTarget::ExplicitSpace(s.spaces[rank])
                                    )
                                    .unwrap(),
                                ParkingCommandOutcome::Committed(_)
                            ));
                        }
                    } else if parked_tick == Some(tick.wrapping_sub(1)) {
                        leave(s, 0);
                    }
                }
                let should_park = arrived.iter().all(|x| *x) && parked_tick.is_none();
                if should_park {
                    parked_tick = Some(tick);
                }
                if parked_tick == Some(tick.wrapping_sub(1)) {
                    left = true;
                }
                let result = paired_tick(&mut a, &mut b, tick == 1);
                for event in result.parking_arrivals() {
                    let c = (0..cells)
                        .find(|c| a.handles[c * 100] == event.vehicle)
                        .expect("only declared probe arrives");
                    assert!(!arrived[c], "arrival must occur once");
                    assert_eq!(
                        event.target,
                        ParkingTarget::ExplicitSpace(a.spaces[c * 100])
                    );
                    arrived[c] = true;
                }
                if tick == 0 {
                    a.population(24, 75, 1);
                } else if parked_tick == Some(tick) {
                    a.population(24, 76, 0);
                } else {
                    a.population(25, 75, 0);
                }
            }
            assert!(arrived.iter().all(|x| *x) && left);
            equal(&a, &b);
            println!(
                "W3 cells={cells} dt_ms={dt} park_observation_tick={} retry=fresh",
                parked_tick.unwrap()
            );
        }
    }
}
