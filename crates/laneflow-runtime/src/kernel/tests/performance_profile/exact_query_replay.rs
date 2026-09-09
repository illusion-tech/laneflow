//! #216 同一固定输入上的批量查询回放；每批一对时钟，不逐车计时。
//! 输入预先收集，缓存访问顺序不同于真实运动循环，因此不等同于生产独占成本。

use super::{CASES, Fixtures, with_candidate};
use crate::kernel::tables::remaining_to_route_end;
use crate::kernel::tick::leader_query_horizon;
use std::hint::black_box;
use std::time::Instant;

#[test]
#[ignore = "manual release #216 isolated query-batch replay, not exclusive step time"]
fn occupancy_exact_query_replay() {
    let fixtures = Fixtures::new();
    for scene in CASES {
        for round in 0..3 {
            with_candidate(false, || {
                let mut harness = fixtures.world(scene);
                harness.validate(scene);
                let mut nanos = [0; 4];
                let mut inspections = 0;
                let mut walks = 0;
                let mut records = 0;
                for _ in 0..64 {
                    harness.world_mut().rebuild_occupancy_index().unwrap();
                    {
                        let world = harness.world();
                        let read = world.read_view();
                        let lengths = world.traffic().lane_lengths_millimetres();
                        let limits = world.traffic().lane_speed_limits_millimetres_per_second();
                        let inputs: Vec<_> = world
                            .live_vehicles()
                            .iter()
                            .map(|handle| {
                                let state = *world.vehicle_state(*handle).unwrap();
                                let compiled = world.compiled_route(state.route).unwrap();
                                let profile = world
                                    .traffic()
                                    .relations()
                                    .vehicle_profile(state.profile)
                                    .unwrap();
                                let horizon =
                                    leader_query_horizon(state.speed_mm_s, profile, 0.1).unwrap();
                                (state, compiled, profile, horizon)
                            })
                            .collect();
                        assert_eq!(inputs.len(), scene.count());
                        let started = Instant::now();
                        for (state, _, _, _) in &inputs {
                            let state = black_box(state);
                            let compiled = read.compiled_route(state.route).unwrap();
                            let cursor = state.route_edge_index as usize;
                            let edge = compiled.edges[cursor];
                            let profile = world
                                .traffic()
                                .relations()
                                .vehicle_profile(state.profile)
                                .unwrap();
                            black_box((
                                compiled.edges.as_slice(),
                                cursor,
                                edge,
                                profile.desired_speed_mm_s().min(limits[edge.index()]),
                                profile,
                            ));
                        }
                        nanos[0] += started.elapsed().as_nanos();

                        let started = Instant::now();
                        for (state, _, profile, _) in &inputs {
                            let (speed, profile, delta) =
                                black_box((state.speed_mm_s, *profile, 0.1));
                            black_box(leader_query_horizon(speed, profile, delta).unwrap());
                        }
                        nanos[1] += started.elapsed().as_nanos();

                        let started = Instant::now();
                        for (state, compiled, _, horizon) in &inputs {
                            let (state, edges, horizon) =
                                black_box((state, compiled.edges.as_slice(), *horizon));
                            black_box(world.derived.occupancy.leader_gap(
                                state.handle,
                                edges,
                                state.route_edge_index as usize,
                                state.progress_mm,
                                black_box(lengths),
                                horizon,
                            ));
                        }
                        nanos[2] += started.elapsed().as_nanos();

                        let started = Instant::now();
                        for (state, compiled, _, _) in &inputs {
                            let (state, compiled) = black_box((state, *compiled));
                            let cursor = state.route_edge_index as usize;
                            black_box((
                                remaining_to_route_end(
                                    compiled.remaining_to_end[cursor],
                                    state.progress_mm,
                                ),
                                read.signal_stop_distance(compiled, state, cursor),
                                read.parking_stop_distance(
                                    compiled,
                                    state,
                                    cursor,
                                    read.committed.parking.binding(state.handle),
                                )
                                .unwrap(),
                            ));
                        }
                        nanos[3] += started.elapsed().as_nanos();
                        records += world.derived.occupancy.records_len();
                        inspections += world.derived.occupancy.inspections();
                        walks += world.derived.occupancy.occurrence_walks();
                    }
                    harness.step();
                }
                harness.validate(scene);
                println!(
                    "exact-replay scene={} round={round} batches=64 queries={} route_profile_ns={} horizon_ns={} leader_gap_ns={} route_stop_ns={} records={records} inspections={inspections} occurrence_walks={walks} digest={}",
                    scene.name(),
                    scene.count() * 64,
                    nanos[0],
                    nanos[1],
                    nanos[2],
                    nanos[3],
                    harness.digest(),
                );
            });
        }
    }
}
