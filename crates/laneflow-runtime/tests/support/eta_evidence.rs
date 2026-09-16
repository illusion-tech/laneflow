//! #676：相同输入的生产库 release 对照；局部重复冲突路线，不代表城市性能。
use super::*;
use sha2::{Digest, Sha256};

fn run(repetitions: usize, lead_ms: u64, samples: usize, warmup: usize) {
    let revision =
        compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            true,
            false,
            13.0,
            ConflictPolicyFixture {
                loop_east: true,
                yielding: true,
                gap_values_ms: Some((lead_ms, 500, 0)),
                ..ConflictPolicyFixture::default()
            },
        ));
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(2, 2, (repetitions * 6) as u64, (repetitions * 2) as u64, 4),
    )
    .unwrap();
    let routes = [0_u32, 1].map(|raw| {
        let stream = revision
            .conflict()
            .participant_stream(ParticipantStreamOrdinal::from_raw(raw))
            .unwrap();
        let path = revision
            .traffic()
            .maneuvers()
            .maneuver_path(stream.maneuver_path())
            .unwrap();
        world
            .register_route(RouteRegisterInput::new(path.edges().repeat(if raw == 0 {
                repetitions
            } else {
                1
            })))
            .unwrap()
    });
    let inputs = routes.map(|route| {
        let edge = world.route_edges(route).unwrap()[0];
        VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            world.traffic().lane_lengths_millimetres()[edge.index()] - 1,
            10_000,
        )
    });
    let before = deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap();
    let mut trace = Sha256::new();
    let mut step_ns = Vec::with_capacity(samples);
    for sample in 0..samples + warmup {
        let vehicles = inputs.map(|input| world.spawn_vehicle(input).unwrap());
        let start = Instant::now();
        black_box(world.step(TickInput::new(4)).unwrap());
        let elapsed = start.elapsed().as_nanos();
        assert_eq!(world.live_vehicles().len(), 2);
        assert_eq!(world.latest_conflict_decisions().len(), 2);
        trace.update(
            format!(
                "{:?}|{:?}|{:?}|{:?}",
                vehicles.map(|vehicle| (
                    world.vehicle(vehicle).unwrap(),
                    world.conflict_reservation(vehicle)
                )),
                world.latest_conflict_decisions(),
                world.latest_waiting_decisions(),
                world.latest_transition_events()
            )
            .as_bytes(),
        );
        if sample >= warmup {
            step_ns.push(elapsed);
        }
        for vehicle in vehicles {
            world.despawn_vehicle(vehicle).unwrap();
        }
    }
    let after = deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap();
    let trace: String = trace
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    println!(
        "eta-ab {{\"repetitions\":{repetitions},\"lead_ms\":{lead_ms},\"active\":2,\"worker\":1,\"samples\":{samples},\"warmup\":{warmup},\"before\":\"{before:x}\",\"after\":\"{after:x}\",\"trace\":\"{trace}\",\"step_ns\":{step_ns:?}}}"
    );
}

#[test]
fn eta_workload_smoke() {
    run(64, 500, 4, 2);
    for repetitions in [1, 4, 16, 64] {
        run(repetitions, 1_000_000, 4, 2);
    }
}

#[test]
#[ignore = "manual release A/B; production library without work counters"]
fn eta_release_ab() {
    run(64, 500, 16_384, 512);
    for repetitions in [1, 4, 16, 64] {
        run(repetitions, 1_000_000, 16_384, 512);
    }
}
