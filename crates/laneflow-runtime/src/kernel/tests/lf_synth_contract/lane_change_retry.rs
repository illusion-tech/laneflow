//! #1218 L 族失败重试（换道预约断点）：仅编译进 Runtime 单元测试，不开放生产 failpoint。
//!
//! 失败边界 = 前车（live-order 优先者）预约建立拍；注入断点覆盖换道预约相关断点
//! `AfterGrants`（staged 授予后、提交前——失败丢弃 staged 不建预约）与
//! `AfterTransitions`（crossing 转移产生后——失败零发布）。失败零发布、只重试原
//! step；retry/fresh 对照。T 触时刻的规范快照按 #1217/#1218 面 B 记录
//! （结构化拒绝变体 + 真实资源身份/历史投影），T-free 边界快照逐字节比较。

use std::collections::HashMap;
use std::fs;
use std::num::NonZeroU32;
use std::path::Path;

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_static_contract::{LaneEdgeId, ParkingSpaceId, ParkingSpaceOrdinal, VehicleProfileId};
use laneflow_static_network::*;
use laneflow_synth_generator::{Bundle, InitialState, verify_directory};
use serde_json::{Value, json};

use crate::kernel::tick::{STEP_FAILPOINT, StepFailpoint};
use crate::*;

const T_RESOURCE_IDENTITY: u32 = 0x8000_0000;

struct LaneChange {
    world: TrafficWorld,
    handles: Vec<VehicleHandle>,
    cluster: [VehicleHandle; 2],
    journal: Vec<(u64, u8, u32, u32)>,
}

impl LaneChange {
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
        let lane_change_route = state
            .routes
            .iter()
            .position(|route| route.edge_ids.len() > 2)
            .unwrap();
        let mut world = TrafficWorld::install(
            revision.clone(),
            WorldConfig::new(
                state.vehicles.len() as u32,
                state.routes.len() as u32,
                state.routes.iter().map(|r| r.edge_ids.len() as u64).sum(),
                16,
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
        let spaces: Vec<Option<ParkingSpaceOrdinal>> = state
            .vehicles
            .iter()
            .map(|vehicle| {
                vehicle.parking_space_id.as_ref().map(|id| {
                    identity
                        .ordinal(id.parse::<ParkingSpaceId>().unwrap())
                        .unwrap()
                })
            })
            .collect();
        let mut handles = Vec::new();
        let mut cluster_slots: [Option<VehicleHandle>; 2] = [None, None];
        for (rank, vehicle) in state.vehicles.iter().enumerate() {
            let route = routes[vehicle.route_rank as usize];
            let on_cluster = vehicle.route_rank as usize == lane_change_route;
            let handle = if vehicle.status == "active" {
                let spawn = VehicleSpawnInput::new(
                    profile,
                    route,
                    vehicle.edge_occurrence,
                    vehicle.progress_mm,
                    vehicle.speed_mm_per_s,
                );
                world
                    .spawn_vehicle(if on_cluster {
                        spawn.with_open_entrance()
                    } else {
                        spawn
                    })
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
                        ParkingTarget::ExplicitSpace(spaces[rank].expect("停车车辆空间")),
                    )
                    .unwrap()
                    .vehicle
            };
            if on_cluster {
                cluster_slots[if vehicle.slot == 0 { 0 } else { 1 }] = Some(handle);
            }
            handles.push(handle);
        }
        let cluster = [cluster_slots[0].unwrap(), cluster_slots[1].unwrap()];
        Self {
            world,
            handles,
            cluster,
            journal: Vec::new(),
        }
    }

    fn leader(&self) -> VehicleHandle {
        self.cluster[0]
    }

    fn step(&mut self, dt: u32) -> StepOutcome {
        let outcome = self.world.step(TickInput::new(u64::from(dt))).unwrap();
        self.record_events();
        outcome
    }

    /// 记录换道预约生命周期事件（真实资源身份：Zone ordinal / T 标记）。
    fn record_events(&mut self) {
        for event in self.world.latest_transition_events() {
            let vehicle = event.vehicle();
            let slot = if vehicle == self.cluster[0] {
                0_u8
            } else if vehicle == self.cluster[1] {
                1
            } else {
                continue;
            };
            let (kind, identity) = match event.kind() {
                TrafficTransitionKind::GateCrossed { gate } => (1_u32, gate.raw()),
                TrafficTransitionKind::ReservationAcquired { passage_range } => {
                    (4, passage_identity(&self.world, passage_range))
                }
                TrafficTransitionKind::ConflictEntered { passage } => {
                    (5, address_identity(&passage.address()))
                }
                TrafficTransitionKind::ConflictCleared { passage } => {
                    (6, address_identity(&passage.address()))
                }
                TrafficTransitionKind::ReservationReleased { passage_range } => {
                    (7, passage_identity(&self.world, passage_range))
                }
                TrafficTransitionKind::ManeuverTraversalCompleted { .. } => (8, 0),
                other => panic!("unexpected lane-change transition {other:?}"),
            };
            self.journal.push((event.tick(), slot, kind, identity));
        }
    }

    /// 面 B 投影（#1218）：结构化拒绝变体 + 真实资源身份/历史 + 逐车相位与预约终态。
    fn face_b(&self) -> Value {
        let refused = match self.world.capture_snapshot() {
            Err(SnapshotCaptureError::UnsupportedTransitionState(inner)) => match inner {
                UnsupportedTransitionState::AdmissionHopWithoutGate => "AdmissionHopWithoutGate",
                UnsupportedTransitionState::TransitionPassageRow => "TransitionPassageRow",
                UnsupportedTransitionState::TransitionLagHistory => "TransitionLagHistory",
            },
            Err(other) => panic!("面 B 必须是过渡结构化拒绝：{other:?}"),
            Ok(_) => panic!("T 触时刻保存必须被结构化拒绝（#1217/#1218 面 B）"),
        };
        let vehicles: Vec<Value> = self
            .cluster
            .iter()
            .enumerate()
            .map(|(slot, &handle)| {
                let state = self.world.vehicle(handle).unwrap();
                let phase = match state.maneuver_traversal().map(|t| t.phase()) {
                    None => 0_u32,
                    Some(ManeuverTraversalPhase::PreGate { .. }) => 1,
                    Some(ManeuverTraversalPhase::Committed { .. }) => 2,
                    Some(ManeuverTraversalPhase::Waiting { .. }) => 3,
                    Some(ManeuverTraversalPhase::Clearing { .. }) => 4,
                };
                let held = self.world.conflict_reservation(handle).map(|r| {
                    json!([
                        r.owner() == handle,
                        r.acquired_tick(),
                        r.passage_range().passage_count()
                    ])
                });
                json!({"slot":slot, "phase":phase, "held":held})
            })
            .collect();
        json!({"persistence_refused":refused,
            "vehicles":vehicles,
            "journal":self.journal.iter().map(|(t, s, k, i)| json!([t, s, k, i])).collect::<Vec<_>>(),
            "tick":self.world.tick_index(),"time_ms":self.world.time_ms(),
            "command_cursor":self.world.command_cursor()})
    }
}

fn address_identity(address: &ConflictPassageAddress) -> u32 {
    match address.static_zone() {
        Some(zone) => zone.raw(),
        None => T_RESOURCE_IDENTITY,
    }
}

fn passage_identity(world: &TrafficWorld, range: ConflictPassageRange) -> u32 {
    let locator = world
        .conflict_passage_occurrence_locator(range.route(), range.first_conflict_occurrence_index())
        .unwrap();
    address_identity(&locator.address())
}

fn digest(snapshot: &CapturedSnapshot) -> String {
    deterministic_state_digest(snapshot)
        .unwrap()
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn save(path: &Path, value: &Value) {
    let mut bytes = serde_json::to_vec_pretty(value).unwrap();
    bytes.push(b'\n');
    fs::write(path, bytes).unwrap();
}

/// 边界公开观测（事件行携带真实资源身份 payload；信号行为空）。
fn observation(sample: &LaneChange, outcome: Option<&StepOutcome>) -> Value {
    let ranks: HashMap<_, _> = sample
        .handles
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
                TrafficTransitionKind::GateCrossed { gate } => (1_u32, gate.raw()),
                TrafficTransitionKind::ReservationAcquired { passage_range } => {
                    (4, passage_identity(&sample.world, passage_range))
                }
                TrafficTransitionKind::ConflictEntered { passage } => {
                    (5, address_identity(&passage.address()))
                }
                TrafficTransitionKind::ConflictCleared { passage } => {
                    (6, address_identity(&passage.address()))
                }
                TrafficTransitionKind::ReservationReleased { passage_range } => {
                    (7, passage_identity(&sample.world, passage_range))
                }
                TrafficTransitionKind::ManeuverTraversalCompleted {
                    maneuver_occurrence_index,
                } => (8, maneuver_occurrence_index),
                other => panic!("unexpected lane-change transition {other:?}"),
            };
            json!([
                event.tick(),
                ranks[&event.vehicle()],
                event.vehicle_update_sequence(),
                0,
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
    let outcome = outcome.map(|outcome| {
        json!({"tick":outcome.tick_index(),"time_ms":outcome.time_ms(),"parking_arrivals":[]})
    });
    json!({"tick":sample.world.tick_index(),"time_ms":sample.world.time_ms(),
        "command_cursor":sample.world.command_cursor(),"events":events,"signals":[],
        "outcome":outcome})
}

fn execution_config(value: &str) -> ExecutionConfig {
    ExecutionConfig::new(value.parse::<NonZeroU32>().unwrap())
}

#[test]
#[ignore = "L 族换道失败重试；显式提供 LF_SYNTH_INPUT、LF_SYNTH_OUTPUT、LF_SYNTH_IMPLEMENTATION、LF_SYNTH_WORKERS、LF_SYNTH_FAILPOINT"]
fn lane_change_failed_step_retry() {
    let input = std::env::var_os("LF_SYNTH_INPUT").expect("LF_SYNTH_INPUT");
    let output = std::env::var_os("LF_SYNTH_OUTPUT").expect("LF_SYNTH_OUTPUT");
    let implementation = std::env::var("LF_SYNTH_IMPLEMENTATION").expect("LF_SYNTH_IMPLEMENTATION");
    let workers = std::env::var("LF_SYNTH_WORKERS").expect("LF_SYNTH_WORKERS");
    let failpoint = std::env::var("LF_SYNTH_FAILPOINT").expect("LF_SYNTH_FAILPOINT");
    let point = match failpoint.as_str() {
        "AfterGrants" => StepFailpoint::AfterGrants,
        "AfterTransitions" => StepFailpoint::AfterTransitions,
        other => panic!("L 族注入断点只覆盖换道预约相关断点：{other}"),
    };
    let bundle = verify_directory(Path::new(&input)).unwrap();
    let plan: Value = serde_json::from_slice(&bundle.files()["input-plan.json"]).unwrap();
    let dt = plan["identity"]["fixed_delta_ms"].as_u64().unwrap();
    let end = plan["warm_up_ticks"].as_u64().unwrap() + plan["observation_ticks"].as_u64().unwrap();
    let output = Path::new(&output);
    fs::create_dir_all(output).unwrap();
    let execution = execution_config(&workers);

    // 探针世界定失败边界 = 前车首次入 T 拍（确定性；与正式输入逐字绑定）。
    let boundary = {
        let mut probe = LaneChange::new(&bundle, execution);
        let mut boundary = None;
        for tick in 1..=end {
            probe.step(dt as u32);
            if probe
                .world
                .vehicle(probe.leader())
                .unwrap()
                .route_edge_index()
                >= 1
            {
                boundary = Some(tick);
                break;
            }
        }
        boundary.expect("前车 crossing 拍")
    };

    let mut failure = LaneChange::new(&bundle, execution);
    let mut fresh = LaneChange::new(&bundle, execution);
    for _tick in 1..boundary {
        failure.step(dt as u32);
        fresh.step(dt as u32);
    }
    assert_eq!(failure.world.tick_index(), boundary - 1);
    let pre = failure.world.capture_snapshot().unwrap();
    let pre_bytes = encode_lfrs(&pre);
    let fresh_pre_bytes = encode_lfrs(&fresh.world.capture_snapshot().unwrap());
    fs::write(output.join("pre-command.lfrs"), &pre_bytes).unwrap();
    fs::write(output.join("post-command.lfrs"), &pre_bytes).unwrap();
    fs::write(output.join("fresh-post-command.lfrs"), &fresh_pre_bytes).unwrap();
    save(
        &output.join("observations.json"),
        &json!({"format":"lf-synth-retry-observations-lc-1",
            "pre_command":observation(&failure, None),
            "post_command":observation(&failure, None),
            "fresh_post_command":observation(&fresh, None),
            "failed":observation(&failure, None),
            "retry":Value::Null,"fresh":Value::Null,"final":Value::Null}),
    );

    let cursor_before_commands = failure.world.command_cursor();
    // 注入失败：零发布（状态/事件/游标不变），failpoint 消耗。
    STEP_FAILPOINT.with(|slot| slot.set(Some(point)));
    let error = failure
        .world
        .step(TickInput::new(dt))
        .expect_err("注入失败拍必须失败");
    assert_eq!(error, StepError::ParkingObservationAllocFailed);
    assert!(
        STEP_FAILPOINT.with(|slot| slot.get().is_none()),
        "failpoint must be consumed"
    );
    let cursor_after_commands = failure.world.command_cursor();
    assert_eq!(failure.world.tick_index(), boundary - 1);
    assert!(failure.world.latest_transition_events().is_empty());
    let failed_snapshot = failure.world.capture_snapshot().unwrap();
    let failed_digest = digest(&failed_snapshot);
    let failed_bytes = encode_lfrs(&failed_snapshot);
    assert_eq!(failed_bytes, pre_bytes, "失败零发布");

    // 只重试原 step；fresh 世界同步推进。
    let retry_outcome = failure.step(dt as u32);
    let fresh_outcome = fresh.step(dt as u32);
    assert_eq!(retry_outcome, fresh_outcome);
    let cursor_after_retry = failure.world.command_cursor();
    assert_eq!(
        failure.world.latest_transition_events(),
        fresh.world.latest_transition_events()
    );
    let retry_face_b = failure.face_b();
    let fresh_face_b = fresh.face_b();
    assert_eq!(retry_face_b, fresh_face_b, "retry/fresh 面 B 记录相等");
    fs::write(output.join("failed.lfrs"), &failed_bytes).unwrap();
    save(&output.join("retry.json"), &retry_face_b);
    save(&output.join("fresh.json"), &fresh_face_b);

    let mut observations =
        serde_json::from_slice::<Value>(&fs::read(output.join("observations.json")).unwrap())
            .unwrap();
    observations["retry"] = observation(&failure, Some(&retry_outcome));
    observations["fresh"] = observation(&fresh, Some(&fresh_outcome));
    save(&output.join("observations.json"), &observations);

    // 续跑至窗口终点（覆盖败者清空后进入）；逐拍 retry/fresh 等价。
    for _tick in (boundary + 1)..=end {
        let a = failure.step(dt as u32);
        let b = fresh.step(dt as u32);
        assert_eq!(a, b);
        assert_eq!(
            failure.world.latest_transition_events(),
            fresh.world.latest_transition_events()
        );
    }
    let final_cursor = failure.world.command_cursor();
    let final_face_b = failure.face_b();
    assert_eq!(final_face_b, fresh.face_b(), "末态面 B 记录相等");
    save(&output.join("final.json"), &final_face_b);
    observations["final"] = observation(&failure, None);
    save(&output.join("observations.json"), &observations);

    save(
        &output.join("report.json"),
        &json!({"format":"lf-synth-lane-change-retry-1","complete":true,
            "implementation_commit":implementation,"workers":workers.parse::<u32>().unwrap(),
            "input_manifest":serde_json::to_value(bundle.manifest()).unwrap(),
            "failure_boundary":boundary - 1,"failed_step_tick":boundary,
            "failpoint":failpoint,"error":"ParkingObservationAllocFailed",
            "failpoint_consumed":true,"failed_calls":1,
            "zero_publication":true,"retry_equals_fresh":true,
            "pre_command_state_digest":digest(&pre),
            "failed_state_digest":failed_digest,
            "warm_up_ticks":plan["warm_up_ticks"],"observation_ticks":plan["observation_ticks"],
            "final_tick":end,
            "cursor_before_commands":cursor_before_commands,"cursor_after_commands":cursor_after_commands,"cursor_after_retry":cursor_after_retry,"final_cursor":final_cursor,
            "retry_face_b":retry_face_b,"final_face_b":final_face_b}),
    );
}
