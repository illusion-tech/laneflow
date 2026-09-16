//! #677：正式生产库的非入口 Gate 输出对照，不代表城市工作负载。
use super::*;
use sha2::{Digest, Sha256};

fn revision(count: usize) -> Arc<SharedNetworkRevision> {
    compile_revision_with_limits(CompileLimits::single_network_1m_v2(), |module| {
        add_standard_profiles(module);
        let mut gates = Vec::new();
        for index in 0..count {
            let [entry, inside, exit, junction, movement, path, gate, stop] = [
                "entry", "inside", "exit", "junction", "movement", "path", "gate", "stop",
            ]
            .map(|suffix| format!("output-{index}-{suffix}"));
            for (edge, successor) in [(&entry, &inside), (&inside, &exit), (&exit, &entry)] {
                module
                    .add_lane_edge(LaneEdgeInput {
                        lane_edge_key: edge,
                        length_meters: 20.0,
                        speed_limit_meters_per_second: 13.75,
                        successors: &[LaneEdgeReference::local(successor)],
                    })
                    .unwrap();
            }
            module
                .add_junction(JunctionInput {
                    junction_key: &junction,
                })
                .unwrap()
                .add_movement(MovementInput {
                    movement_key: &movement,
                    junction: JunctionReference::local(&junction),
                    directed_entry_approach_key: "in",
                    directed_exit_approach_key: "out",
                    turn_direction: None,
                })
                .unwrap()
                .add_maneuver_path(ManeuverPathInput {
                    maneuver_path_key: &path,
                    movement: MovementReference::local(&movement),
                    entry_edge: LaneEdgeReference::local(&entry),
                    internal_edges: &[LaneEdgeReference::local(&inside)],
                    exit_edge: LaneEdgeReference::local(&exit),
                })
                .unwrap()
                .add_stop_line(StopLineInput {
                    stop_line_key: &stop,
                    lane_edge: LaneEdgeReference::local(&entry),
                })
                .unwrap()
                .add_maneuver_gate(ManeuverGateInput {
                    maneuver_gate_key: &gate,
                    maneuver_path: ManeuverPathReference::local(&path),
                    transition_index: 0,
                    stop_line: StopLineReference::local(&stop),
                    signal_control: SignalControlInput::None,
                })
                .unwrap();
            gates.push(gate);
        }
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "completed",
                length_meters: 20.0,
                speed_limit_meters_per_second: 13.75,
                successors: &[],
            })
            .unwrap();
        test_policy::add_gate_policy(
            module,
            "signal-policy",
            &gates
                .iter()
                .map(|gate| {
                    (
                        gate.as_str(),
                        laneflow_compiler::GateInterpretation::Uncontrolled,
                    )
                })
                .collect::<Vec<_>>(),
        );
    })
}

fn run(
    revision: &Arc<SharedNetworkRevision>,
    active: usize,
    completed: usize,
    crossing: usize,
    samples: usize,
    warmup: usize,
) {
    let mut world = install_fixture(
        Arc::clone(revision),
        WorldConfig::new(
            (active + completed) as u32,
            (active + 1) as u32,
            (active * 3 + 1) as u64,
            1,
            1,
            100,
        ),
    )
    .unwrap();
    let completed_route = register_named(&mut world, &["completed"]);
    for _ in 0..completed {
        let vehicle = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                completed_route,
                0,
                20_000,
                0,
            ))
            .unwrap();
        world.step(TickInput::new(100)).unwrap();
        assert_eq!(
            world.vehicle(vehicle).unwrap().status(),
            VehicleStatus::Completed
        );
    }
    let routes = (0..active)
        .map(|index| {
            let keys = ["entry", "inside", "exit"].map(|suffix| format!("output-{index}-{suffix}"));
            register_named(
                &mut world,
                &keys.iter().map(String::as_str).collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    let before = deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap();
    let mut trace = Sha256::new();
    let mut times = Vec::with_capacity(samples);
    for sample in 0..samples + warmup {
        let vehicles = routes
            .iter()
            .enumerate()
            .map(|(index, route)| {
                world
                    .spawn_vehicle(VehicleSpawnInput::new(
                        VehicleProfileOrdinal::from_raw(0),
                        *route,
                        0,
                        if index < crossing { 19_999 } else { 0 },
                        10_000,
                    ))
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let start = Instant::now();
        black_box(world.step(TickInput::new(100)).unwrap());
        let elapsed = start.elapsed().as_nanos();
        if sample >= warmup {
            times.push(elapsed);
        }
        assert_eq!(world.latest_waiting_decisions().len(), crossing);
        assert!(
            world
                .latest_waiting_decisions()
                .iter()
                .all(|d| d.zone().is_none())
        );
        assert_eq!(world.live_vehicles().len(), active + completed);
        trace.update(
            format!(
                "{:?}|{:?}|{:?}|{:?}",
                vehicles
                    .iter()
                    .map(|v| world.vehicle(*v).unwrap())
                    .collect::<Vec<_>>(),
                world.latest_waiting_decisions(),
                world.latest_conflict_decisions(),
                world.latest_transition_events(),
            )
            .as_bytes(),
        );
        for vehicle in vehicles {
            world.despawn_vehicle(vehicle).unwrap();
        }
    }
    let after = deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap();
    let trace: String = trace
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    println!(
        "waiting-output-ab {{\"active\":{active},\"live\":{},\"crossing\":{crossing},\"samples\":{samples},\"warmup\":{warmup},\"before\":\"{before:x}\",\"after\":\"{after:x}\",\"trace\":\"{trace}\",\"step_ns\":{times:?}}}",
        active + completed
    );
}

#[test]
fn waiting_output_smoke() {
    let revision = revision(4);
    for crossing in [0, 1, 4] {
        run(&revision, 4, 3, crossing, 4, 2);
    }
}

#[test]
#[ignore = "manual production release A/B without library work counters"]
fn waiting_output_release_ab() {
    let revision = revision(512);
    for active in [64, 512] {
        for crossing in [0, 1, active] {
            run(&revision, active, 0, crossing, 256, 32);
        }
    }
    for crossing in [0, 1, 64] {
        run(&revision, 64, 1_536, crossing, 256, 32);
    }
}
