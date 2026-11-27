use std::fs;
use std::num::NonZeroU32;

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::*;
use laneflow_static_contract::{
    EntityKind, LaneEdgeId, ParkingSpaceId, RightOfWayPolicySetId, SignalControllerId,
    VehicleProfileId,
};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};
use serde_json::{Value, json};

use crate::bundle::generate_inputs;
use crate::encoding::{canonical_json, config_bytes, digest, hex};
use crate::runtime as handoff;
use crate::*;

fn provenance() -> Provenance {
    Provenance {
        source_commit: "a".repeat(40),
        generator_commit: "b".repeat(40),
    }
}

fn lane_change_case() -> Case {
    Case::LaneChange(LaneChangeParams {
        clusters: 1,
        changers_per_cluster: 2,
    })
}

fn probe(cells: u32, case: Case, delta: u32) -> Identity {
    let mut identity = Identity::new(10_000, case, delta).unwrap();
    identity.scale = cells * 100;
    identity
}

#[test]
fn canonical_encoding_has_an_external_sha256_vector() {
    let bytes = canonical_json(&json!({"b":2,"a":1})).unwrap();
    assert_eq!(bytes, b"{\"a\":1,\"b\":2}\n");
    // #1156：固定向量由 .NET SHA256.HashData 独立核对。
    assert_eq!(
        hex(&digest(&bytes)),
        "e8d38819d39f705646bfb643368eca78f7db476c16471dbc33b941b27326410d"
    );
    assert!(canonical_json(&json!({"x":1.5})).is_err());
    assert_eq!(
        canonical_json(&json!({"z":{"z":0,"a":["/月",true,null]},"a":-1})).unwrap(),
        "{\"a\":-1,\"z\":{\"a\":[\"/月\",true,null],\"z\":0}}\n".as_bytes()
    );
    let config = config_bytes(&Identity::new(10_000, Case::W1, 16).unwrap()).unwrap();
    assert_eq!(config, b"{\"format\":\"lf-synth-config-1\",\"identity\":{\"case\":\"W1\",\"fixed_delta_ms\":16,\"scale\":10000,\"seed\":0,\"workload_id\":\"LF-SYNTH-v2\"}}\n");
}

#[test]
fn formal_identity_is_closed() {
    assert!(Identity::new(100, Case::W1, 16).is_err());
    assert!(Identity::new(10_000, Case::W1, 17).is_err());
    let mut identity = Identity::new(10_000, Case::W1, 16).unwrap();
    identity.seed = 1;
    assert!(generate(identity, provenance()).is_err());
    assert!(
        generate(
            Identity::new(10_000, Case::W1, 16).unwrap(),
            Provenance {
                source_commit: "HEAD".into(),
                generator_commit: "b".repeat(40),
            }
        )
        .is_err()
    );
    // #1243 T3：反序列化或改写 `case` 字段的身份与构造器同一可行域——
    // 每簇只接受两条外侧源路线，`generate` 经 `Identity::validate` 同样拒绝。
    let out_of_domain = Case::LaneChange(LaneChangeParams {
        clusters: 1,
        changers_per_cluster: 3,
    });
    let mut value =
        serde_json::to_value(Identity::new(10_000, lane_change_case(), 16).unwrap()).unwrap();
    let params = value["case"]["RouteHandoff"].clone();
    assert_eq!(params, json!({"clusters": 1, "changers_per_cluster": 2}));
    let mut old = value.clone();
    old["case"] = json!({"LaneChange": params});
    assert!(serde_json::from_value::<Identity>(old).is_err());
    value["case"] = serde_json::to_value(&out_of_domain).unwrap();
    let bypassed: Identity = serde_json::from_value(value).unwrap();
    assert!(generate(bypassed, provenance()).is_err());
}

#[test]
fn geometry_covers_all_cells_with_exact_split_and_disjoint_bands() {
    for cell in 0..1_000 {
        for route in 0..4 {
            let edges = network::centerlines(cell, route);
            for edge in &edges {
                assert_eq!(edge.len(), 52);
                let length: f64 = edge
                    .windows(2)
                    .map(|p| {
                        (f64::from(p[1].x) - f64::from(p[0].x)).abs()
                            + (f64::from(p[1].z) - f64::from(p[0].z)).abs()
                    })
                    .sum();
                assert_eq!(length, 5_130.0, "axis aligned telescoping length");
                let committed_arc = edge.windows(2).fold(0.0_f32, |sum, p| {
                    sum + (p[1].x - p[0].x)
                        .hypot(p[1].y - p[0].y)
                        .hypot(p[1].z - p[0].z)
                });
                assert_eq!(committed_arc, 5_130.0, "production f32 arc accumulation");
                for point in edge {
                    assert!((-16_384.0..=16_384.0).contains(&point.x));
                    assert!((-16_384.0..=16_384.0).contains(&point.z));
                    assert_eq!(point.y, 0.0);
                }
            }
            let split = edges[0].last().unwrap();
            assert_eq!(split.x, edges[1][0].x);
            assert_eq!(split.z, edges[1][0].z);
            assert_eq!(split.x, (-16_200 + 1_024 * i64::from(cell % 32)) as f32);
            assert_eq!(
                split.z,
                (-16_270 + 1_024 * i64::from(cell / 32) + 80 * i64::from(route)) as f32
            );
            assert_eq!(edges[1].last().unwrap().z - edges[0][0].z, 60.0);
        }
    }
}

#[test]
fn state_and_plans_freeze_population_windows_and_command_order() {
    for dt in [16, 33] {
        for case in [Case::W1, Case::W2, Case::W3, Case::W4] {
            let identity = probe(2, case.clone(), dt);
            let state = state::initial_state(&identity, &[]).unwrap();
            assert_eq!(state.routes.len(), 8);
            assert_eq!(state.vehicles.len(), 200);
            let expected = match &case {
                Case::W1 | Case::W4 => 150,
                Case::W2 => 200,
                Case::W3 => 50,
                Case::LaneChange(_) => unreachable!("换道族另有探针冻结"),
            };
            assert_eq!(
                state
                    .vehicles
                    .iter()
                    .filter(|v| v.status == "active")
                    .count(),
                expected
            );
            for (rank, vehicle) in state.vehicles.iter().enumerate() {
                assert_eq!(vehicle.logical_rank as usize, rank);
                assert_eq!(vehicle.route_rank, vehicle.logical_rank / 25);
                assert_eq!(vehicle.slot, vehicle.logical_rank % 25);
                if vehicle.status == "parked" {
                    assert_eq!(vehicle.edge_occurrence, 1);
                    assert_eq!(vehicle.progress_mm, 5_000 + vehicle.slot * 7_500);
                    assert_eq!(vehicle.speed_mm_per_s, 0);
                }
            }
            let plan = plan::input_plan(&identity);
            let warm = if dt == 16 { 14_508 } else { 7_044 };
            assert_eq!(plan["warm_up_ticks"], warm);
            assert_eq!(
                plan["observation_ticks"],
                if case == Case::W4 { 2 } else { warm * 2 }
            );
            let scheduled = plan["scheduled"].as_array().unwrap();
            match case {
                Case::W1 | Case::W2 => assert!(scheduled.is_empty()),
                Case::W3 => {
                    assert_eq!(scheduled[0]["boundary"], 0);
                    assert_eq!(scheduled[0]["commands"][1]["logical_rank"], 100);
                    assert_eq!(scheduled[1]["boundary"], warm);
                    assert_eq!(scheduled[1]["commands"][0]["operation"], "despawn");
                    assert_eq!(scheduled[1]["commands"][1]["logical_rank"], 81);
                    assert_eq!(scheduled[1]["commands"][1]["progress_mm"], EDGE_MM);
                    assert_eq!(scheduled[1]["commands"][3]["logical_rank"], 181);
                }
                Case::W4 => {
                    assert_eq!(scheduled[0]["boundary"], warm - 1);
                    assert_eq!(scheduled[1]["boundary"], warm);
                    let commands = scheduled[1]["commands"].as_array().unwrap();
                    assert_eq!(
                        commands
                            .iter()
                            .map(|c| c["logical_rank"].as_u64().unwrap())
                            .collect::<Vec<_>>(),
                        [87, 187, 62, 162]
                    );
                    assert_eq!(commands[0]["operation"], "atomic_replace");
                    assert_eq!(commands[2]["operation"], "leave_parking");
                    assert_eq!(scheduled[2]["boundary"], warm + 1);
                    assert_eq!(scheduled[2]["commands"][1]["progress_mm"], 5_129_900);
                }
                Case::LaneChange(_) => assert!(scheduled.is_empty()),
            }
        }
    }
}

#[test]
fn generated_inputs_install_and_spawn_through_production_apis() {
    let mut vectors = Vec::new();
    for dt in [16, 33] {
        for case in [Case::W1, Case::W2, Case::W3, Case::W4] {
            let identity = probe(2, case, dt);
            let bundle = generate_inputs(identity.clone(), provenance()).unwrap();
            vectors.push(json!({"identity":identity,"files":bundle.manifest().files}));
            assert_runtime_consumption(&bundle);
            let again = generate_inputs(identity, provenance()).unwrap();
            assert_eq!(bundle.files(), again.files());
        }
    }
    assert_eq!(
        canonical_json(&vectors).unwrap(),
        include_bytes!("../fixtures/probe-vectors.json")
    );
}

pub(crate) fn assert_runtime_consumption(bundle: &Bundle) {
    let manifest = bundle.manifest();
    let input = check_canonical_network_input(
        bundle.files()["network.lfca"].as_slice(),
        FormatLimits::HARD,
    )
    .unwrap();
    let revision = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(2 * 1_024 * 1_024 * 1_024, 2 * 1_024 * 1_024 * 1_024),
        ),
    )
    .unwrap();
    for (name, kind) in [
        ("edges", EntityKind::LaneEdge),
        ("controllers", EntityKind::SignalController),
        ("groups", EntityKind::SignalGroup),
        ("gates", EntityKind::ManeuverGate),
        ("stop_lines", EntityKind::StopLine),
        ("junctions", EntityKind::Junction),
        ("movements", EntityKind::Movement),
        ("maneuver_paths", EntityKind::ManeuverPath),
        ("facilities", EntityKind::ParkingFacility),
        ("spaces", EntityKind::ParkingSpace),
    ] {
        assert_eq!(
            revision.traffic().entity_counts().count(kind),
            manifest.counts[name]
        );
    }
    assert!(
        revision
            .traffic()
            .lane_lengths_millimetres()
            .iter()
            .all(|&length| length == EDGE_MM)
    );
    assert!(
        revision
            .traffic()
            .lane_speed_limits_millimetres_per_second()
            .iter()
            .all(|&speed| speed == 13_900)
    );
    let state: InitialState = serde_json::from_slice(&bundle.files()["state.json"]).unwrap();
    let identity = &manifest.identity;
    let expected_phases: [u64; 6] = if identity.fixed_delta_ms == 16 {
        [30_000, 3_008, 1_008, 20_000, 3_008, 1_008]
    } else {
        [30_030, 3_003, 1_023, 20_031, 3_003, 1_023]
    };
    let cycle: u64 = expected_phases.iter().sum();
    for cell in 0..identity.cells() {
        let id =
            state::stable_id(EntityKind::SignalController, &key(cell, 0, "controller")).unwrap();
        let ordinal = revision
            .identity()
            .ordinal(id.parse::<SignalControllerId>().unwrap())
            .unwrap();
        let controller = revision
            .traffic()
            .relations()
            .signal_controller(ordinal)
            .unwrap();
        assert_eq!(controller.cycle_ms(), cycle);
        assert_eq!(
            controller.offset_ms(),
            if identity.case == Case::W4 {
                cycle - u64::from(identity.fixed_delta_ms)
            } else {
                u64::from(cell) * 997 % cycle
            }
        );
        let actual: Vec<_> = controller
            .phases()
            .iter()
            .map(|&phase| {
                revision
                    .traffic()
                    .relations()
                    .signal_phase(phase)
                    .unwrap()
                    .duration_ms()
            })
            .collect();
        assert_eq!(actual, expected_phases);
    }
    let spatial = revision.spatial().unwrap().lane_pose().unwrap();
    let mut point_count = 0;
    for route in &state.routes {
        let entry = revision
            .identity()
            .ordinal(route.edge_ids[0].parse::<LaneEdgeId>().unwrap())
            .unwrap();
        let exit = revision
            .identity()
            .ordinal(route.edge_ids[1].parse::<LaneEdgeId>().unwrap())
            .unwrap();
        assert_eq!(revision.traffic().successors(entry).unwrap(), &[exit]);
        assert!(revision.traffic().successors(exit).unwrap().is_empty());
        let expected = network::centerlines(route.cell, route.route);
        for (edge, expected_points) in [entry, exit].into_iter().zip(expected) {
            let geometry = spatial.lane_geometry(edge).unwrap();
            assert_eq!(geometry.points().len(), 52);
            point_count += geometry.points().len() as u32;
            for (point, expected) in geometry.points().iter().zip(expected_points) {
                assert_eq!(
                    [point.x, point.y, point.z],
                    [expected.x, expected.y, expected.z]
                );
            }
        }
    }
    assert_eq!(point_count, manifest.counts["geometry_points"]);
    let origin = *revision.canonical_origin();
    let profile = revision
        .identity()
        .ordinal(
            state
                .vehicle_profile_id
                .parse::<VehicleProfileId>()
                .unwrap(),
        )
        .unwrap();
    let mut world = TrafficWorld::install(laneflow_runtime::WorldInstallInput {
        revision: revision.clone(),
        config: WorldConfig::try_new(laneflow_runtime::WorldConfigInput {
            vehicle_capacity: manifest.identity.scale,
            route_capacity: state.routes.len() as u32,
            route_edge_occurrence_capacity: state.routes.len() as u64 * 2,
            route_conflict_occurrence_capacity: 0,
            fixed_delta_time_ms: u64::from(manifest.identity.fixed_delta_ms),
        })
        .expect("有效的交通配置"),
        execution: ExecutionConfig::new(NonZeroU32::MIN),
        source: CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "fixture://lf-synth-generator",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .unwrap(),
        },
        world_id: laneflow_runtime::WorldId::new(0),
        policy_selection: WorldPolicySelection::Pinned(PolicyPin {
            policy: state.policy_id.parse::<RightOfWayPolicySetId>().unwrap(),
        }),
    })
    .unwrap();
    let routes: Vec<_> = state
        .routes
        .iter()
        .map(|route| {
            let edges: Vec<_> = route
                .edge_ids
                .iter()
                .map(|id| {
                    revision
                        .identity()
                        .ordinal(id.parse::<LaneEdgeId>().unwrap())
                        .unwrap()
                })
                .collect();
            world
                .register_route(RouteRegisterInput::new(edges))
                .unwrap()
        })
        .collect();
    for vehicle in &state.vehicles {
        let route = routes[vehicle.route_rank as usize];
        let handle = if vehicle.status == "active" {
            world
                .spawn_vehicle(VehicleSpawnInput::new(
                    profile,
                    route,
                    vehicle.edge_occurrence,
                    vehicle.progress_mm,
                    vehicle.speed_mm_per_s,
                ))
                .unwrap()
        } else {
            let space = revision
                .identity()
                .ordinal(
                    vehicle
                        .parking_space_id
                        .as_ref()
                        .unwrap()
                        .parse::<ParkingSpaceId>()
                        .unwrap(),
                )
                .unwrap();
            world
                .spawn_parked_vehicle(
                    ParkedVehicleSpawnInput::new(
                        profile,
                        route,
                        vehicle.edge_occurrence,
                        vehicle.progress_mm,
                    ),
                    ParkingTarget::ExplicitSpace(space),
                )
                .unwrap()
                .vehicle
        };
        assert_eq!(
            world.vehicle(handle).unwrap().status(),
            if vehicle.status == "active" {
                VehicleStatus::Active
            } else {
                VehicleStatus::Parked
            }
        );
    }
    world
        .step(TickInput::new(u64::from(manifest.identity.fixed_delta_ms)))
        .unwrap();
    assert_eq!(
        world.live_vehicles().len(),
        manifest.identity.scale as usize
    );
    let active = world
        .live_vehicles()
        .iter()
        .filter(|&&h| world.vehicle(h).unwrap().status() == VehicleStatus::Active)
        .count();
    assert_eq!(active as u32, manifest.counts["active"]);
}

#[test]
fn verifier_rejects_rehashed_state_order_and_preserves_existing_output() {
    let bundle = generate(Identity::new(10_000, Case::W1, 16).unwrap(), provenance()).unwrap();
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("bundle");
    bundle.write_new(&dir).unwrap();
    assert!(bundle.write_new(&dir).is_err());
    assert_eq!(verify_directory(&dir).unwrap().files(), bundle.files());
    let mut state: Value = serde_json::from_slice(&bundle.files()["state.json"]).unwrap();
    state["vehicles"].as_array_mut().unwrap().swap(0, 1);
    let altered = canonical_json(&state).unwrap();
    fs::write(dir.join("state.json"), &altered).unwrap();
    assert!(
        verify_directory(&dir)
            .unwrap_err()
            .to_string()
            .contains("摘要不符")
    );
    let mut manifest: Manifest = serde_json::from_slice(&bundle.files()["manifest.json"]).unwrap();
    let record = manifest
        .files
        .iter_mut()
        .find(|r| r.name == "state.json")
        .unwrap();
    record.digest = hex(&digest(&altered));
    record.byte_length = altered.len() as u64;
    fs::write(
        dir.join("manifest.json"),
        canonical_json(&manifest).unwrap(),
    )
    .unwrap();
    assert!(
        verify_directory(&dir)
            .unwrap_err()
            .to_string()
            .contains("规范重建")
    );
    fs::write(dir.join("extra"), []).unwrap();
    assert!(
        verify_directory(&dir)
            .unwrap_err()
            .to_string()
            .contains("恰有四个")
    );
}

#[test]
fn invalid_handoff_observation_preserves_the_previous_tick() {
    use crate::handoff::{Reservation, VehicleObservation, observe, requests};

    let identity = probe(1, lane_change_case(), 16);
    let bundle = generate_inputs(identity.clone(), provenance()).unwrap();
    let state: InitialState = serde_json::from_slice(&bundle.files()["state.json"]).unwrap();
    let leader = identity.scale as usize;
    let leader_route = state.vehicles[leader].route_rank;
    let previous = vec![(1, 0, 10, leader_route)];
    let mut phases = [10, 0];
    let mut journal = previous.clone();
    let read = |rank: usize, follower_held: bool| VehicleObservation {
        cursor: u32::from(rank == leader),
        front_mm: state.vehicles[rank].progress_mm,
        handoff: (rank == leader || follower_held).then_some(Reservation {
            source_index: 0,
            acquired_tick: if rank == leader { 1 } else { 2 },
            front_handed_off: rank == leader,
        }),
    };
    let error = observe(
        &state,
        2,
        state.vehicles.len(),
        |rank, _| Ok(read(rank, true)),
        &mut phases,
        &mut journal,
    )
    .unwrap_err();
    assert!(error.contains("两车同时持有"));
    assert_eq!(phases, [10, 0]);
    assert_eq!(journal, previous);
    observe(
        &state,
        2,
        state.vehicles.len(),
        |rank, _| Ok(read(rank, false)),
        &mut phases,
        &mut journal,
    )
    .unwrap();
    assert_eq!(phases, [11, 0]);
    assert_eq!(
        journal,
        [(1, 0, 10, leader_route), (2, 0, 11, leader_route)]
    );

    let error = requests(
        &state,
        0,
        state.vehicles.len(),
        |rank, _| {
            Ok(VehicleObservation {
                cursor: 0,
                front_mm: state.vehicles[rank].progress_mm - 1,
                handoff: None,
            })
        },
        |_| Ok(()),
    )
    .unwrap_err();
    assert!(error.contains("换道前杠退至出生位之前"));
}

#[test]
fn lane_change_lifecycle_probe_freezes_window_pinning() {
    for dt in [16_u32, 33] {
        let identity = probe(2, lane_change_case(), dt);
        let bundle = generate_inputs(identity.clone(), provenance()).unwrap();
        let again = generate_inputs(identity.clone(), provenance()).unwrap();
        assert_eq!(bundle.files(), again.files(), "L1 规范重建逐字节确定");
        let (warm, observation) = identity.lane_change_window();
        let end = warm + observation;
        let input = check_canonical_network_input(
            bundle.files()["network.lfca"].as_slice(),
            FormatLimits::HARD,
        )
        .unwrap();
        let revision = build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::RetainAvailable,
                SharedNetworkBuildLimits::new(2 * 1_024 * 1_024 * 1_024, 2 * 1_024 * 1_024),
            ),
        )
        .unwrap();
        let state: InitialState = serde_json::from_slice(&bundle.files()["state.json"]).unwrap();
        let edge_occurrences: u64 = state.routes.iter().map(|r| r.edge_ids.len() as u64).sum();
        let origin = *revision.canonical_origin();
        let profile = revision
            .identity()
            .ordinal(
                state
                    .vehicle_profile_id
                    .parse::<VehicleProfileId>()
                    .unwrap(),
            )
            .unwrap();
        let mut world = TrafficWorld::install(laneflow_runtime::WorldInstallInput {
            revision: revision.clone(),
            config: handoff::configure_world(
                &state,
                WorldConfig::try_new(laneflow_runtime::WorldConfigInput {
                    vehicle_capacity: state.vehicles.len() as u32,
                    route_capacity: state.routes.len() as u32,
                    route_edge_occurrence_capacity: edge_occurrences,
                    route_conflict_occurrence_capacity: 16,
                    fixed_delta_time_ms: u64::from(identity.fixed_delta_ms),
                })
                .unwrap(),
            )
            .unwrap(),
            execution: ExecutionConfig::new(NonZeroU32::MIN),
            source: CommittedNetworkSource::Published {
                reference: PublishedLfcaReference::new(
                    "fixture://lf-synth-generator",
                    origin.canonical_artifact_digest(),
                    origin.canonical_artifact_byte_length(),
                    origin.network_revision(),
                )
                .unwrap(),
            },
            world_id: laneflow_runtime::WorldId::new(0),
            policy_selection: WorldPolicySelection::Pinned(PolicyPin {
                policy: state.policy_id.parse::<RightOfWayPolicySetId>().unwrap(),
            }),
        })
        .unwrap();
        let routes: Vec<_> = state
            .routes
            .iter()
            .map(|route| handoff::register_route(&mut world, route).unwrap())
            .collect();
        let mut changers = Vec::new();
        let mut handles = Vec::new();
        for vehicle in &state.vehicles {
            let route = routes[vehicle.route_rank as usize];
            let lane_change_route = state.routes[vehicle.route_rank as usize].handoff.is_some();
            let handle = if vehicle.status == "active" {
                let spawn = VehicleSpawnInput::new(
                    profile,
                    route,
                    vehicle.edge_occurrence,
                    vehicle.progress_mm,
                    vehicle.speed_mm_per_s,
                );
                world.spawn_vehicle(spawn).unwrap()
            } else {
                let space = revision
                    .identity()
                    .ordinal(
                        vehicle
                            .parking_space_id
                            .as_ref()
                            .unwrap()
                            .parse::<ParkingSpaceId>()
                            .unwrap(),
                    )
                    .unwrap();
                world
                    .spawn_parked_vehicle(
                        ParkedVehicleSpawnInput::new(
                            profile,
                            route,
                            vehicle.edge_occurrence,
                            vehicle.progress_mm,
                        ),
                        ParkingTarget::ExplicitSpace(space),
                    )
                    .unwrap()
                    .vehicle
            };
            handles.push(handle);
            if lane_change_route {
                changers.push(handle);
            }
        }
        assert_eq!(changers.len(), 2);
        let mut phases = [0; 2];
        let mut journal = Vec::new();
        for tick in 1..=end {
            let requests = handoff::requests(&world, &state, &handles, &routes).unwrap();
            match tick {
                1 => assert_eq!(requests.len(), 2, "同拍在两个源车道上争用中间车道"),
                2 => {
                    assert_eq!(requests.len(), 1);
                    assert_eq!(requests[0].vehicle, changers[1]);
                }
                3 => assert!(requests.is_empty()),
                _ => {}
            }
            world
                .step_with_route_handoffs(TickInput::new(u64::from(dt)), &requests)
                .unwrap_or_else(|error| panic!("dt={dt}, tick={tick}: {error:?}"));
            if tick == 2 {
                assert!(world.vehicle_route_handoffs(changers[1]).next().is_none());
                assert!(world.vehicle_route_handoffs(changers[0]).next().is_some());
            }
            handoff::observe(&world, &state, &handles, &routes, &mut phases, &mut journal).unwrap();
            assert!(
                changers
                    .iter()
                    .all(|&handle| world.vehicle(handle).unwrap().status()
                        != VehicleStatus::Completed),
                "dt={dt}, tick={tick}"
            );
            if tick == warm || tick == end - 1 {
                let active = changers
                    .iter()
                    .filter(|&&handle| world.vehicle_route_handoffs(handle).next().is_some())
                    .count();
                assert_eq!(
                    active,
                    usize::from(tick == warm),
                    "dt={dt}, tick={tick}, journal={journal:?}"
                );
                world
                    .capture_snapshot()
                    .expect("活动期间及清空后均可捕获快照");
            }
        }
        // 逐拍比较同一输入的实际进入、前杠交接和整车清空。阻塞的第二车在源道上重试。
        let ticks = if dt == 16 {
            [1, 243, 632, 995, 1_480, 2_071]
        } else {
            [1, 118, 306, 483, 718, 1_005]
        };
        let routes = &state.vehicles[identity.scale as usize..];
        let expected: Vec<_> = ticks
            .into_iter()
            .enumerate()
            .map(|(i, tick)| {
                let slot = i / 3;
                (
                    tick,
                    slot as u16,
                    10 + (i % 3) as u32,
                    routes[slot].route_rank,
                )
            })
            .collect();
        assert_eq!(journal, expected, "dt={dt}");
        assert!(ticks[5] < end - 1);
    }
}

#[test]
fn lane_change_multi_cluster_is_deterministic_and_counted() {
    for clusters in [1_u32, 4, 16] {
        let identity = Identity::new(
            10_000,
            Case::LaneChange(LaneChangeParams {
                clusters,
                changers_per_cluster: 2,
            }),
            16,
        )
        .unwrap();
        let bundle = generate_inputs(identity.clone(), provenance()).unwrap();
        let again = generate_inputs(identity.clone(), provenance()).unwrap();
        assert_eq!(
            bundle.files(),
            again.files(),
            "簇数 {clusters} 规范重建逐字节确定"
        );
        let state: InitialState = serde_json::from_slice(&bundle.files()["state.json"]).unwrap();
        assert_eq!(
            state.routes.len() as u32,
            identity.cells() * 4 + clusters * 2
        );
        assert_eq!(state.vehicles.len() as u32, identity.scale + clusters * 2);
        let plan: Value = serde_json::from_slice(&bundle.files()["input-plan.json"]).unwrap();
        let competition = plan["rules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|rule| rule["kind"] == "lane_change_competition")
            .unwrap();
        assert_eq!(competition["clusters"], clusters);
        assert_eq!(competition["registered_handoffs"], 2 * clusters);
        assert_eq!(
            bundle.manifest().counts["routes"],
            state.routes.len() as u32
        );
        assert_eq!(
            competition["changers"].as_array().unwrap().len() as u32,
            clusters * 2
        );
    }
}

#[test]
fn each_handoff_cluster_has_two_outer_lane_routes() {
    for slots in [1, 3, 4, 8, 64] {
        assert!(
            Identity::new(
                10_000,
                Case::LaneChange(LaneChangeParams {
                    clusters: 1,
                    changers_per_cluster: slots,
                }),
                16
            )
            .is_err()
        );
    }
    let identity = probe(1, lane_change_case(), 16);
    let bundle = generate_inputs(identity.clone(), provenance()).unwrap();
    let state: InitialState = serde_json::from_slice(&bundle.files()["state.json"]).unwrap();
    let cluster: Vec<_> = state
        .routes
        .iter()
        .filter(|r| r.handoff.is_some())
        .collect();
    assert_eq!(cluster.len(), 2);
    let windows = [cluster[0], cluster[1]].map(|r| r.handoff.as_ref().unwrap());
    assert_eq!(windows[0].section_id, windows[1].section_id);
    assert_eq!(windows.map(|w| w.left_index), [0, 1]);
    assert_ne!(cluster[0].edge_ids[0], cluster[1].edge_ids[0]);
    assert_eq!(cluster[0].edge_ids[1..], cluster[1].edge_ids[1..]);
    let checked = check_canonical_network_input(
        bundle.files()["network.lfca"].as_slice(),
        FormatLimits::HARD,
    )
    .unwrap();
    let revision = build_shared_network_revision(
        checked,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(2 << 30, 2 << 30),
        ),
    )
    .unwrap();
    let counts = revision.traffic().entity_counts();
    for (name, kind) in [
        ("edges", EntityKind::LaneEdge),
        ("gates", EntityKind::ManeuverGate),
        ("stop_lines", EntityKind::StopLine),
        ("junctions", EntityKind::Junction),
        ("movements", EntityKind::Movement),
        ("maneuver_paths", EntityKind::ManeuverPath),
    ] {
        assert_eq!(counts.count(kind), bundle.manifest().counts[name], "{name}");
    }
}

mod full_scale;
