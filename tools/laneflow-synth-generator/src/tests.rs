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
            let state = state::initial_state(&identity).unwrap();
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
    let mut world = TrafficWorld::install(
        revision.clone(),
        WorldConfig::new(
            manifest.identity.scale,
            state.routes.len() as u32,
            state.routes.len() as u64 * 2,
            0,
            u64::from(manifest.identity.fixed_delta_ms),
        ),
        ExecutionConfig::new(NonZeroU32::MIN),
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "fixture://lf-synth-generator",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .unwrap(),
        },
        0,
        WorldPolicySelection::Pinned(PolicyPin {
            policy: state.policy_id.parse::<RightOfWayPolicySetId>().unwrap(),
        }),
    )
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
fn lane_change_lifecycle_probe_freezes_window_pinning() {
    // L1 探针（#1218 D3）：钉版换道生命周期拍号并冻结窗口不变量——
    // 检查点（16/32 拍）必须早于首 crossing；生命周期收束必须早于窗口终点；
    // 两换道者路线完成必须晚于窗口终点（观察窗内无 Completed）。
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
        let mut world = TrafficWorld::install(
            revision.clone(),
            WorldConfig::new(
                state.vehicles.len() as u32,
                state.routes.len() as u32,
                edge_occurrences,
                16,
                u64::from(identity.fixed_delta_ms),
            ),
            ExecutionConfig::new(NonZeroU32::MIN),
            CommittedNetworkSource::Published {
                reference: PublishedLfcaReference::new(
                    "fixture://lf-synth-generator",
                    origin.canonical_artifact_digest(),
                    origin.canonical_artifact_byte_length(),
                    origin.network_revision(),
                )
                .unwrap(),
            },
            0,
            WorldPolicySelection::Pinned(PolicyPin {
                policy: state.policy_id.parse::<RightOfWayPolicySetId>().unwrap(),
            }),
        )
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
        let mut changers = Vec::new();
        for vehicle in &state.vehicles {
            let route = routes[vehicle.route_rank as usize];
            let lane_change_route = vehicle.route_rank == state.routes.len() as u32 - 1
                && identity.lane_change().is_some();
            let handle = if vehicle.status == "active" {
                let spawn = VehicleSpawnInput::new(
                    profile,
                    route,
                    vehicle.edge_occurrence,
                    vehicle.progress_mm,
                    vehicle.speed_mm_per_s,
                );
                world
                    .spawn_vehicle(if lane_change_route {
                        spawn.with_open_entrance()
                    } else {
                        spawn
                    })
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
            if lane_change_route {
                changers.push(handle);
            }
        }
        assert_eq!(changers.len(), 2);
        let mut leader_enter = None;
        let mut follower_enter = None;
        let mut follower_release = None;
        let mut refusal_onset = None;
        let mut completed = None;
        let mut checkpoints_capturable = Vec::new();
        let mut leader_release = None;
        let mut leader_exit_t = None;
        let mut follower_exit_t = None;
        let mut leader_completed = None;
        let mut follower_completed = None;
        for tick in 1..=1_200 {
            world
                .step(TickInput::new(u64::from(identity.fixed_delta_ms)))
                .unwrap();
            let leader = world.vehicle(changers[0]).unwrap();
            let follower = world.vehicle(changers[1]).unwrap();
            if leader.route_edge_index() >= 1 && leader_enter.is_none() {
                leader_enter = Some(tick);
            }
            if follower.route_edge_index() >= 1 && follower_enter.is_none() {
                follower_enter = Some(tick);
            }
            if leader.route_edge_index() >= 2 && leader_exit_t.is_none() {
                leader_exit_t = Some(tick);
            }
            if follower.route_edge_index() >= 2 {
                follower_exit_t.get_or_insert(tick);
            }
            if leader.status() == VehicleStatus::Completed {
                leader_completed.get_or_insert(tick);
            }
            if follower.status() == VehicleStatus::Completed {
                follower_completed.get_or_insert(tick);
            }
            if tick <= end
                && (leader.status() == VehicleStatus::Completed
                    || follower.status() == VehicleStatus::Completed)
                && completed.is_none()
            {
                completed = Some(tick);
            }
            match world.capture_snapshot() {
                Ok(_) => {
                    if tick == warm || tick == 2 * warm {
                        checkpoints_capturable.push(tick);
                    }
                }
                Err(_) => {
                    refusal_onset.get_or_insert(tick);
                }
            };
            for event in world.latest_transition_events() {
                let released = matches!(
                    event.kind(),
                    TrafficTransitionKind::ReservationReleased { .. }
                );
                if released && event.vehicle() == changers[0] {
                    leader_release.get_or_insert(tick);
                }
                if released && event.vehicle() == changers[1] {
                    follower_release.get_or_insert(tick);
                }
            }
        }
        eprintln!(
            "L1 probe dt={dt}: warm={warm} planned_end={end}\n  leader: enter={leader_enter:?} \
             exit_t={leader_exit_t:?} release={leader_release:?} completed={leader_completed:?}\n  \
             follower: enter={follower_enter:?} exit_t={follower_exit_t:?} \
             release={follower_release:?} completed={follower_completed:?}\n  \
             refusal_onset={refusal_onset:?} checkpoints={checkpoints_capturable:?} \
             first_completion={completed:?}"
        );
        let leader_enter = leader_enter.expect("leader enters T");
        let follower_enter = follower_enter.expect("follower enters T");
        assert!(
            leader_enter < follower_enter,
            "live-order: leader first ({leader_enter}) then follower ({follower_enter})"
        );
        assert!(
            follower_enter > 2 * warm,
            "checkpoints 16/32 must precede any crossing: follower {follower_enter}"
        );
        assert_eq!(
            checkpoints_capturable,
            vec![warm, 2 * warm],
            "面 A：T-free 检查点可捕获"
        );
        assert_eq!(
            refusal_onset,
            Some(leader_enter),
            "面 B：拒绝面恰在首个 T 裁决拍开始"
        );
        let leader_release = leader_release.expect("leader releases T");
        let follower_release = follower_release.expect("follower releases T");
        assert!(
            follower_enter > leader_release,
            "败者在前车预约释放后进入（R1b 层②）：{follower_enter} > {leader_release}"
        );
        assert!(
            follower_release > follower_enter,
            "败者进入后须自行释放预约"
        );
        assert!(
            leader_release < end,
            "前车完整生命周期须收束在窗口内：{leader_release} < {end}"
        );
        assert!(
            follower_enter < end,
            "败者清空后进入拍须落在窗口内：{follower_enter} < {end}"
        );
        assert!(
            completed.is_none(),
            "窗口内不得出现换道者路线完成（完成拍 {completed:?} 须晚于终点 {end}）"
        );
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
        let state = state::initial_state(&identity).unwrap();
        assert_eq!(state.routes.len() as u32, identity.cells() * 4 + clusters);
        assert_eq!(state.vehicles.len() as u32, identity.scale + clusters * 2);
        let plan: Value = serde_json::from_slice(&bundle.files()["input-plan.json"]).unwrap();
        let competition = plan["rules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|rule| rule["kind"] == "lane_change_competition")
            .unwrap();
        assert_eq!(competition["transition_segments"], clusters);
        assert_eq!(
            competition["changers"].as_array().unwrap().len() as u32,
            clusters * 2
        );
    }
}

#[test]
fn lane_change_entry_geometry_scales_with_changer_count() {
    use crate::{lane_change_entry_length_mm, lane_change_slot_progress_mm};
    // K≤4 保持 30 m 钉版入口（既有证据逐字不变）。
    for k in [2_u32, 3, 4] {
        assert_eq!(lane_change_entry_length_mm(k), 30_000, "K={k} 入口长度");
    }
    // K≥5 延长入口并以 body+gap=7,000 mm 精确间距排列，全体不相交。
    for k in [5_u32, 8, 16, 64] {
        let entry = lane_change_entry_length_mm(k);
        let mut previous: Option<u32> = None;
        for slot in 0..k {
            let progress = lane_change_slot_progress_mm(k, slot);
            assert!(progress < entry as u32, "K={k} 位置越界");
            if let Some(prev) = previous {
                assert!(prev - progress >= 7_000, "K={k} slot={slot} 间距不足");
            }
            previous = Some(progress);
        }
        assert_eq!(previous, Some(3_000), "K={k} 末车位置不确定");
    }
    // K=8：生成状态无重叠且网络可编译。
    let identity = Identity::new(
        10_000,
        Case::LaneChange(LaneChangeParams {
            clusters: 1,
            changers_per_cluster: 8,
        }),
        16,
    )
    .unwrap();
    let state = state::initial_state(&identity).unwrap();
    let mut progresses: Vec<u32> = state
        .vehicles
        .iter()
        .filter(|v| v.parking_space_id.is_none())
        .map(|v| v.progress_mm)
        .collect();
    progresses.sort_unstable();
    for pair in progresses.windows(2) {
        assert!(pair[1] - pair[0] >= 7_000, "K=8 初始车体重叠");
    }
    let _ = generate_inputs(identity, provenance()).unwrap();
}

mod full_scale;
