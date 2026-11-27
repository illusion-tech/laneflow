//! L1 同拍横移申请的故障注入。逐拍对照独立世界，保存进入拍及末态的规范快照。
#[path = "../../../../../../tools/laneflow-synth-harness/src/handoff.rs"]
mod handoff;

use std::collections::HashMap;
use std::fs;
use std::num::NonZeroU32;
use std::path::Path;

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_static_contract::{
    ParkingSpaceId, ParkingSpaceOrdinal, SignalAspect, VehicleProfileId,
};
use laneflow_static_network::*;
use laneflow_synth_generator::{Bundle, InitialState, verify_directory};
use serde_json::{Value, json};

use crate::kernel::tick::{STEP_FAILPOINT, StepFailpoint};
use crate::*;

struct LaneChange {
    world: TrafficWorld,
    handles: Vec<VehicleHandle>,
    routes: Vec<RouteHandle>,
    phases: Vec<u32>,
    journal: Vec<(u64, u16, u32, u32)>,
    state: InitialState,
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
        let mut world = TrafficWorld::install(crate::WorldInstallInput {
            revision: revision.clone(),
            config: handoff::configure_world(
                &state,
                WorldConfig::try_new(crate::WorldConfigInput {
                    vehicle_capacity: state.vehicles.len() as u32,
                    route_capacity: state.routes.len() as u32,
                    route_edge_occurrence_capacity: state
                        .routes
                        .iter()
                        .map(|r| r.edge_ids.len() as u64)
                        .sum(),
                    route_conflict_occurrence_capacity: 16,
                    fixed_delta_time_ms: u64::from(state.identity.fixed_delta_ms),
                })
                .expect("有效的交通配置"),
            )
            .unwrap(),
            execution,
            source: CommittedNetworkSource::Published { reference: source },
            world_id: crate::WorldId::new(0),
            policy_selection: WorldPolicySelection::Pinned(PolicyPin {
                policy: state.policy_id.parse().unwrap(),
            }),
        })
        .unwrap();
        let routes: Vec<_> = state
            .routes
            .iter()
            .map(|route| handoff::register_route(&mut world, route).unwrap())
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

        for (rank, vehicle) in state.vehicles.iter().enumerate() {
            let route = routes[vehicle.route_rank as usize];
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
            handles.push(handle);
        }
        let slots = state.vehicles.len() - state.identity.scale as usize;
        Self {
            world,
            handles,
            routes,
            phases: vec![0; slots],
            journal: Vec::new(),
            state,
        }
    }

    fn requests(&self) -> Vec<RouteHandoffRequest> {
        handoff::requests(&self.world, &self.state, &self.handles, &self.routes).unwrap()
    }

    fn step(&mut self, dt: u32) -> StepOutcome {
        let requests = self.requests();
        let outcome = self
            .world
            .step_with_route_handoffs(TickInput::new(u64::from(dt)), &requests)
            .unwrap();
        handoff::observe(
            &self.world,
            &self.state,
            &self.handles,
            &mut self.phases,
            &mut self.journal,
        )
        .unwrap();
        outcome
    }
}

fn address_identity(address: &ConflictPassageAddress) -> u32 {
    address
        .static_zone()
        .expect("L1 路口事件引用实际冲突区")
        .raw()
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

/// 采集边界的公开观测，事件行携带路口资源身份。
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
                sample.state.vehicles[ranks[&event.vehicle()]].route_rank,
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
                    panic!("unexpected LF-SYNTH parking target")
                };
                json!([ranks[&arrival.vehicle], space.raw()])
            })
            .collect();
        json!({"tick":outcome.tick_index(),"time_ms":outcome.time_ms(),"parking_arrivals":arrivals})
    });
    json!({"tick":sample.world.tick_index(),"time_ms":sample.world.time_ms(),
        "command_cursor":sample.world.command_cursor(),"events":events,"signals":signals,
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

    // 以首车实际进入拍定位故障边界，两份原始世界独立重建同批申请。
    let boundary = {
        let mut probe = LaneChange::new(&bundle, execution);
        (1..=end)
            .find(|_| {
                probe.step(dt as u32);
                probe.phases.first() == Some(&10)
            })
            .expect("首车获准进入横移")
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
    let mut observations = json!({"format":"lf-synth-retry-observations-handoff-1",
        "pre_command":observation(&failure, None),"post_command":observation(&failure, None),
        "fresh_post_command":observation(&fresh, None),"failed":Value::Null,
        "retry":Value::Null,"fresh":Value::Null,"final":Value::Null});
    save(&output.join("observations.json"), &observations);

    // L1 申请随 step 提交，此处没有独立的主机命令。
    let command_cursor = failure.world.command_cursor();
    // 注入失败：零发布（状态/事件/游标不变），failpoint 消耗。
    STEP_FAILPOINT.with(|slot| slot.set(Some(point)));
    let requests = failure.requests();
    assert_eq!(requests.len(), failure.phases.len(), "全部换道者同拍竞争");
    let error = failure
        .world
        .step_with_route_handoffs(TickInput::new(dt), &requests)
        .expect_err("注入失败拍必须失败");
    observations["failed"] = observation(&failure, None);
    save(&output.join("observations.json"), &observations);
    let failed_snapshot = failure.world.capture_snapshot().unwrap();
    let failed_digest = digest(&failed_snapshot);
    let failed_bytes = encode_lfrs(&failed_snapshot);
    fs::write(output.join("failed.lfrs"), &failed_bytes).unwrap();
    assert_eq!(error, StepError::ParkingObservationAllocFailed);
    assert!(
        STEP_FAILPOINT.with(|slot| slot.get().is_none()),
        "failpoint must be consumed"
    );
    assert_eq!(failure.world.tick_index(), boundary - 1);
    assert_eq!(observations["post_command"], observations["failed"]);
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
    assert_eq!(failure.phases[0], 10, "首车的重试确实建立横移预约");
    assert_eq!(failure.phases[1], 0, "竞争者没有同拍获得预约");
    let retry_snapshot = failure.world.capture_snapshot().unwrap();
    let retry_bytes = encode_lfrs(&retry_snapshot);
    let fresh_bytes = encode_lfrs(&fresh.world.capture_snapshot().unwrap());
    assert_eq!(retry_bytes, fresh_bytes, "重试拍活动横移快照");
    assert_eq!(failure.journal, fresh.journal);
    fs::write(output.join("retry.lfrs"), &retry_bytes).unwrap();
    fs::write(output.join("fresh.lfrs"), &fresh_bytes).unwrap();

    observations["retry"] = observation(&failure, Some(&retry_outcome));
    observations["fresh"] = observation(&fresh, Some(&fresh_outcome));
    save(&output.join("observations.json"), &observations);

    // 续跑至窗口终点（覆盖败者清空后进入）；逐拍 retry/fresh 等价。
    let mut final_outcome = None;
    for _tick in (boundary + 1)..=end {
        let a = failure.step(dt as u32);
        let b = fresh.step(dt as u32);
        assert_eq!(a, b);
        assert_eq!(
            failure.world.latest_transition_events(),
            fresh.world.latest_transition_events()
        );
        final_outcome = Some(a);
    }
    let final_cursor = failure.world.command_cursor();
    let final_snapshot = failure.world.capture_snapshot().unwrap();
    let final_bytes = encode_lfrs(&final_snapshot);
    assert_eq!(
        final_bytes,
        encode_lfrs(&fresh.world.capture_snapshot().unwrap())
    );
    assert_eq!(failure.journal, fresh.journal);
    assert!(failure.phases.iter().all(|&kind| kind == 12));
    fs::write(output.join("final.lfrs"), final_bytes).unwrap();
    observations["final"] = observation(&failure, final_outcome.as_ref());
    save(&output.join("observations.json"), &observations);

    save(
        &output.join("report.json"),
        &json!({"format":"lf-synth-route-handoff-retry-1","complete":true,
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
            "cursor_before_commands":command_cursor,"cursor_after_commands":command_cursor,"cursor_after_retry":cursor_after_retry,"final_cursor":final_cursor,
            "retry_state_digest":digest(&retry_snapshot),"final_state_digest":digest(&final_snapshot),
            "handoff_journal":failure.journal}),
    );
}
