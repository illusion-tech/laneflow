//! #1162 正式输入的独立安装与失败重试。仅编译进 Runtime 单元测试，不开放生产 failpoint。

use std::collections::HashMap;
use std::fs;
use std::num::NonZeroU32;
use std::path::Path;

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_static_contract::{
    LaneEdgeId, ParkingSpaceId, ParkingSpaceOrdinal, SignalAspect, VehicleProfileId,
    VehicleProfileOrdinal,
};
use laneflow_static_network::*;
use laneflow_synth_generator::{Bundle, Case, InitialState, verify_directory};
use serde_json::{Value, json};

use crate::kernel::tick::{STEP_FAILPOINT, StepFailpoint};
use crate::*;

struct Formal {
    world: TrafficWorld,
    handles: Vec<VehicleHandle>,
    routes: Vec<RouteHandle>,
    spaces: Vec<ParkingSpaceOrdinal>,
    profile: VehicleProfileOrdinal,
}

impl Formal {
    fn new(bundle: &Bundle, execution: ExecutionConfig) -> Self {
        let checked = check_canonical_network_input(
            bundle.files()["network.lfca"].as_slice(),
            FormatLimits::HARD,
        )
        .unwrap();
        let revision = build_shared_network_revision(
            checked,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::RetainAvailable,
                SharedNetworkBuildLimits::new(2 * 1_024 * 1_024 * 1_024, 2 * 1_024 * 1_024 * 1_024),
            ),
        )
        .unwrap();
        let state: InitialState = serde_json::from_slice(&bundle.files()["state.json"]).unwrap();
        let identity = revision.identity();
        let profile = identity
            .ordinal(
                state
                    .vehicle_profile_id
                    .parse::<VehicleProfileId>()
                    .unwrap(),
            )
            .unwrap();
        let origin = *revision.canonical_origin();
        let source = PublishedLfcaReference::new(
            "fixture://lf-synth-harness",
            origin.canonical_artifact_digest(),
            origin.canonical_artifact_byte_length(),
            origin.network_revision(),
        )
        .unwrap();
        let mut world = TrafficWorld::install(
            revision.clone(),
            WorldConfig::new(
                state.identity.scale,
                state.routes.len() as u32,
                state.routes.len() as u64 * 2,
                0,
                u64::from(state.identity.fixed_delta_ms),
            ),
            execution,
            CommittedNetworkSource::Published { reference: source },
            0,
            WorldPolicySelection::Pinned(PolicyPin {
                policy: state.policy_id.parse().unwrap(),
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
                    .map(|id| identity.ordinal(id.parse::<LaneEdgeId>().unwrap()).unwrap())
                    .collect();
                world
                    .register_route(RouteRegisterInput::new(edges))
                    .unwrap()
            })
            .collect();
        let spaces: Vec<_> = state
            .vehicles
            .iter()
            .map(|vehicle| {
                identity
                    .ordinal(vehicle.parking_space_id.parse::<ParkingSpaceId>().unwrap())
                    .unwrap()
            })
            .collect();
        let handles = state
            .vehicles
            .iter()
            .map(|vehicle| {
                let route = routes[vehicle.route_rank as usize];
                if vehicle.status == "active" {
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
                    world
                        .spawn_parked_vehicle(
                            ParkedVehicleSpawnInput::new(
                                profile,
                                route,
                                vehicle.edge_occurrence,
                                vehicle.progress_mm,
                            ),
                            ParkingTarget::ExplicitSpace(spaces[vehicle.logical_rank as usize]),
                        )
                        .unwrap()
                        .vehicle
                }
            })
            .collect();
        Self {
            world,
            handles,
            routes,
            spaces,
            profile,
        }
    }

    fn apply(&mut self, commands: &[Value]) {
        for command in commands {
            let rank = number(command, "logical_rank") as usize;
            let old = self.handles[rank];
            let route = self.routes[number(command, "route_rank") as usize];
            let input = VehicleSpawnInput::new(
                self.profile,
                route,
                number(command, "edge_occurrence"),
                number(command, "progress_mm"),
                number(command, "speed_mm_per_s"),
            );
            let cursor = self.world.command_cursor();
            match command["operation"].as_str().unwrap() {
                "reserve_parking" => {
                    assert!(matches!(
                        self.world
                            .reserve_parking(
                                old,
                                ReserveParkingTarget::ExplicitSpace {
                                    space: self.spaces[rank],
                                    entry_route_occurrence: input.route_edge_index()
                                }
                            )
                            .unwrap(),
                        ParkingCommandOutcome::Committed(_)
                    ));
                }
                "despawn" => {
                    self.world.despawn_vehicle(old).unwrap();
                }
                "spawn" => {
                    self.handles[rank] = self.world.spawn_vehicle(input).unwrap();
                }
                "atomic_replace" => {
                    assert_eq!(
                        self.world.vehicle(old).unwrap().status(),
                        VehicleStatus::Completed
                    );
                    self.handles[rank] = self
                        .world
                        .replace_completed_vehicle(old, input)
                        .unwrap()
                        .new;
                    assert!(self.world.vehicle(old).is_none());
                }
                "leave_parking" => {
                    self.world
                        .leave_parking(
                            old,
                            LeaveParkingTarget::ExplicitSpace {
                                space: self.spaces[rank],
                                route,
                                exit_route_occurrence: input.route_edge_index(),
                            },
                        )
                        .unwrap();
                }
                other => panic!("unexpected prefix command {other}"),
            }
            assert_eq!(self.world.command_cursor(), cursor + 1);
        }
    }
}

fn number(value: &Value, key: &str) -> u32 {
    u32::try_from(value[key].as_u64().unwrap()).unwrap()
}

fn commands_at(plan: &Value, boundary: u64) -> Vec<Value> {
    plan["scheduled"]
        .as_array()
        .unwrap()
        .iter()
        .find(|batch| batch["boundary"] == boundary)
        .map_or_else(Vec::new, |batch| {
            batch["commands"].as_array().unwrap().clone()
        })
}

fn replacement_commands(plan: &Value) -> Vec<Value> {
    let rule = plan["rules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rule| rule["kind"] == "completed_replacement")
        .unwrap();
    rule["ranks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|rank| {
            json!({"operation":"atomic_replace", "logical_rank":rank,
        "route_rank":rank.as_u64().unwrap()/25, "edge_occurrence":rule["edge_occurrence"],
        "progress_mm":rule["progress_mm"],"speed_mm_per_s":rule["speed_mm_per_s"]})
        })
        .collect()
}

fn batch_digest(commands: &[Value]) -> String {
    let mut bytes = b"lf-synth-command-batch-1\0".to_vec();
    bytes.extend_from_slice(&(commands.len() as u32).to_le_bytes());
    for command in commands {
        let operation: u32 = match command["operation"].as_str().unwrap() {
            "reserve_parking" => 0,
            "park_vehicle" => 1,
            "leave_parking" => 2,
            "despawn" => 3,
            "spawn" => 4,
            "atomic_replace" => 5,
            _ => panic!("command operation"),
        };
        bytes.extend_from_slice(&operation.to_le_bytes());
        for key in [
            "logical_rank",
            "route_rank",
            "edge_occurrence",
            "progress_mm",
            "speed_mm_per_s",
        ] {
            bytes.extend_from_slice(&number(command, key).to_le_bytes());
        }
    }
    blake3::hash(&bytes).to_hex().to_string()
}

fn digest(snapshot: &CapturedSnapshot) -> String {
    deterministic_state_digest(snapshot)
        .unwrap()
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn assert_equal(a: &Formal, b: &Formal) {
    assert_eq!(a.handles, b.handles);
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
    assert_eq!(a.world.command_cursor(), b.world.command_cursor());
}

fn paired_step(a: &mut Formal, b: &mut Formal, dt: u32) -> StepOutcome {
    let outcome = a.world.step(TickInput::new(u64::from(dt))).unwrap();
    assert_eq!(
        outcome,
        b.world.step(TickInput::new(u64::from(dt))).unwrap()
    );
    assert_eq!(
        a.world.latest_transition_events(),
        b.world.latest_transition_events()
    );
    outcome
}

fn execution_config(value: &str) -> Option<ExecutionConfig> {
    value
        .parse::<NonZeroU32>()
        .ok()
        .filter(|count| count.get() <= 16)
        .map(ExecutionConfig::new)
}

// #1191：保存失败边界的完整公开观测字段及其原始顺序；不把布尔证明当成跨 worker 事件比较。
fn observation(sample: &Formal, outcome: Option<&StepOutcome>) -> Value {
    let ranks: HashMap<_, _> = sample
        .handles
        .iter()
        .enumerate()
        .map(|(rank, &h)| (h, rank))
        .collect();
    let routes: HashMap<_, _> = sample
        .routes
        .iter()
        .enumerate()
        .map(|(rank, &h)| (h, rank))
        .collect();
    let events: Vec<_> = sample
        .world
        .latest_transition_events()
        .iter()
        .map(|event| {
            let anchor = event.anchor();
            let (kind, payload) = match event.kind() {
                TrafficTransitionKind::GateCrossed { gate } => (1, gate.raw()),
                TrafficTransitionKind::ManeuverTraversalCompleted {
                    maneuver_occurrence_index,
                } => (8, maneuver_occurrence_index),
                _ => panic!("unexpected LF-SYNTH transition"),
            };
            json!([
                event.tick(),
                ranks[&event.vehicle()],
                event.vehicle_update_sequence(),
                routes[&anchor.route()],
                anchor.maneuver_occurrence_index(),
                anchor.hop(),
                anchor.position().route_edge_index(),
                anchor.position().progress_mm(),
                anchor.position().carry_um(),
                kind,
                payload
            ])
        })
        .collect();
    let signals: Vec<_> = sample
        .world
        .committed_signal_groups()
        .as_slice()
        .iter()
        .map(|(group, aspect)| {
            let aspect = match aspect {
                SignalAspect::Red => 0,
                SignalAspect::Yellow => 1,
                SignalAspect::Green => 2,
                _ => panic!("unexpected LF-SYNTH signal aspect"),
            };
            json!([group.raw(), aspect])
        })
        .collect();
    let outcome = outcome.map(|outcome| {
        let arrivals: Vec<_> = outcome
            .parking_arrivals()
            .iter()
            .map(|arrival| {
                let ParkingTarget::ExplicitSpace(space) = arrival.target else {
                    panic!("unexpected parking target")
                };
                json!([ranks[&arrival.vehicle], space.raw()])
            })
            .collect();
        json!({"tick":outcome.tick_index(),"time_ms":outcome.time_ms(),"parking_arrivals":arrivals})
    });
    json!({"tick":sample.world.tick_index(),"time_ms":sample.world.time_ms(),"command_cursor":sample.world.command_cursor(),
        "events":events,"signals":signals,"outcome":outcome})
}

fn save(path: &Path, value: &Value) {
    let mut bytes = serde_json::to_vec_pretty(value).unwrap();
    bytes.push(b'\n');
    fs::write(path, bytes).unwrap();
}

#[test]
#[ignore = "正式一万/十万 guardrail；显式提供 LF_SYNTH_INPUT、LF_SYNTH_OUTPUT、LF_SYNTH_IMPLEMENTATION、LF_SYNTH_WORKERS"]
fn formal_input_failed_step_retry() {
    #[allow(
        clippy::assertions_on_constants,
        reason = "显式拒绝以普通测试构建出具正式重试证据"
    )]
    {
        assert!(!cfg!(debug_assertions), "optimized test binary required");
    }
    let input = std::env::var_os("LF_SYNTH_INPUT").expect("LF_SYNTH_INPUT");
    let output = std::env::var_os("LF_SYNTH_OUTPUT").expect("LF_SYNTH_OUTPUT");
    let implementation = std::env::var("LF_SYNTH_IMPLEMENTATION").expect("LF_SYNTH_IMPLEMENTATION");
    let execution = execution_config(&std::env::var("LF_SYNTH_WORKERS").expect("LF_SYNTH_WORKERS"))
        .expect("worker count in 1..=16");
    assert!(
        implementation.len() == 40
            && implementation
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    );
    let output = Path::new(&output);
    fs::create_dir(output).unwrap();
    save(
        &output.join("report.json"),
        &json!({"format":"lf-synth-retry-run-1","complete":false,"implementation_commit":implementation,"workers":execution.worker_count().get()}),
    );
    let bundle = verify_directory(Path::new(&input)).unwrap();
    let replay_bundle = verify_directory(Path::new(&input)).unwrap();
    let identity = &bundle.manifest().identity;
    assert!(matches!(identity.case, Case::W3 | Case::W4));
    let plan: Value = serde_json::from_slice(&bundle.files()["input-plan.json"]).unwrap();
    let warm = plan["warm_up_ticks"].as_u64().unwrap();
    let dt = identity.fixed_delta_ms;
    let failure_boundary = warm + u64::from(identity.case == Case::W3);
    let mut a = Formal::new(&bundle, execution);
    let mut b = Formal::new(&replay_bundle, execution);
    assert_eq!(a.world.execution_config(), execution);
    assert_eq!(b.world.execution_config(), execution);
    assert_equal(&a, &b);
    for boundary in 0..failure_boundary {
        let commands = commands_at(&plan, boundary);
        a.apply(&commands);
        b.apply(&commands);
        paired_step(&mut a, &mut b, dt);
        if boundary % 1_024 == 0 {
            eprintln!(
                "retry {:?} scale={} dt={dt} tick={}/{failure_boundary}",
                identity.case,
                identity.scale,
                boundary + 1
            );
        }
    }
    assert_equal(&a, &b);
    let pre = a.world.capture_snapshot().unwrap();
    let mut observations =
        json!({"format":"lf-synth-retry-observations-1","pre_command":observation(&a, None)});
    fs::write(output.join("pre-command.lfrs"), encode_lfrs(&pre)).unwrap();
    let commands = if identity.case == Case::W3 {
        replacement_commands(&plan)
    } else {
        commands_at(&plan, failure_boundary)
    };
    let cursor_before = a.world.command_cursor();
    a.apply(&commands);
    b.apply(&commands);
    assert_equal(&a, &b);
    let before = a.world.capture_snapshot().unwrap();
    let before_digest = digest(&before);
    let events = a.world.latest_transition_events().to_vec();
    let signals = a.world.committed_signal_groups();
    let cursor = a.world.command_cursor();
    observations["post_command"] = observation(&a, None);
    observations["fresh_post_command"] = observation(&b, None);
    fs::write(output.join("post-command.lfrs"), encode_lfrs(&before)).unwrap();
    fs::write(
        output.join("fresh-post-command.lfrs"),
        encode_lfrs(&b.world.capture_snapshot().unwrap()),
    )
    .unwrap();
    let mut step_bytes = b"lf-synth-step-input-1\0".to_vec();
    step_bytes.extend_from_slice(&u64::from(dt).to_le_bytes());
    STEP_FAILPOINT.set(Some(StepFailpoint::AfterTransitions));
    assert_eq!(
        a.world.step(TickInput::new(u64::from(dt))),
        Err(StepError::ParkingObservationAllocFailed)
    );
    assert!(STEP_FAILPOINT.get().is_none(), "failpoint must be consumed");
    let failed = a.world.capture_snapshot().unwrap();
    assert_eq!(failed, before);
    assert_eq!(digest(&failed), before_digest);
    assert_eq!(a.world.latest_transition_events(), events);
    assert_eq!(a.world.committed_signal_groups(), signals);
    assert_eq!(a.world.command_cursor(), cursor);
    observations["failed"] = observation(&a, None);
    assert_eq!(observations["post_command"], observations["failed"]);
    fs::write(output.join("failed.lfrs"), encode_lfrs(&failed)).unwrap();
    let mut final_outcome = paired_step(&mut a, &mut b, dt);
    assert_equal(&a, &b);
    observations["retry"] = observation(&a, Some(&final_outcome));
    observations["fresh"] = observation(&b, Some(&final_outcome));
    assert_eq!(observations["retry"], observations["fresh"]);
    assert_eq!(
        a.world.command_cursor(),
        cursor,
        "retry must not repeat commands"
    );
    let retried = a.world.capture_snapshot().unwrap();
    fs::write(output.join("retry.lfrs"), encode_lfrs(&retried)).unwrap();
    fs::write(
        output.join("fresh.lfrs"),
        encode_lfrs(&b.world.capture_snapshot().unwrap()),
    )
    .unwrap();
    if identity.case == Case::W4 {
        let b1_commands = commands_at(&plan, warm + 1);
        a.apply(&b1_commands);
        b.apply(&b1_commands);
        final_outcome = paired_step(&mut a, &mut b, dt);
        assert_equal(&a, &b);
    }
    assert_eq!(a.world.tick_index(), warm + 2);
    let final_state = a.world.capture_snapshot().unwrap();
    fs::write(output.join("final.lfrs"), encode_lfrs(&final_state)).unwrap();
    observations["final"] = observation(&a, Some(&final_outcome));
    save(&output.join("observations.json"), &observations);
    save(
        &output.join("report.json"),
        &json!({"format":"lf-synth-retry-run-1","complete":true,
        "scope":"optimized_test_only_retry_semantics_not_performance_or_product_certification",
        "implementation_commit":implementation,"input_manifest":bundle.manifest(),"workers":a.world.execution_config().worker_count().get(),
        "failure_boundary":failure_boundary,"final_tick":a.world.tick_index(),"final_cursor":a.world.command_cursor(),
        "pre_command_state_digest":digest(&pre),"post_command_state_digest":before_digest,
        "command_batch_digest":batch_digest(&commands),"command_batch":commands,
        "step_input_digest":blake3::hash(&step_bytes).to_hex().to_string(),
        "cursor_before_commands":cursor_before,"cursor_after_commands":cursor,"cursor_after_retry":cursor,
        "failpoint":"AfterTransitions","error":"ParkingObservationAllocFailed","failed_calls":1,
        "failed_snapshot_values_equal":true,"failed_events_equal":true,"failed_cursor_equal":true,"failed_signals_equal":true,
        "failpoint_consumed":true,"fresh_execution_config_matches":true,
        "fresh_post_command_values_equal":true,"retry_fresh_values_equal":true,"retry_fresh_outcome_and_events_equal":true,
        "retry_state_digest":digest(&retried),"final_state_digest":digest(&final_state)}),
    );
}

#[test]
fn retry_execution_config_requires_explicit_supported_workers() {
    for count in [1, 2, 4, 8, 16] {
        assert_eq!(
            execution_config(&count.to_string())
                .unwrap()
                .worker_count()
                .get(),
            count
        );
    }
    for count in ["", "0", "17", "-1", "invalid", "4294967296"] {
        assert!(execution_config(count).is_none());
    }
}
