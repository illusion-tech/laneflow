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
            let identity = probe(2, case, dt);
            let state = state::initial_state(&identity).unwrap();
            assert_eq!(state.routes.len(), 8);
            assert_eq!(state.vehicles.len(), 200);
            let expected = match case {
                Case::W1 | Case::W4 => 150,
                Case::W2 => 200,
                Case::W3 => 50,
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
                .ordinal(vehicle.parking_space_id.parse::<ParkingSpaceId>().unwrap())
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

mod full_scale;
