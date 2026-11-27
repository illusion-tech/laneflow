//! 用正式 L1 首车的路网、出生状态与首次申请驱动一个单车世界，逐拍核对无头与 Bevy。
use std::{num::NonZeroU32, time::Duration};

use bevy_app::App;
use bevy_math::Vec3;
use bevy_time::Time;
use bevy_transform::components::Transform;
use laneflow_bevy::{
    LaneFlowAdapterError, LaneFlowCommittedPoseBatch, LaneFlowPlugin, LaneFlowSession,
    LaneFlowSessionConfig,
};
use laneflow_runtime::{
    CommittedHandoffGeometry, ExecutionConfig, RouteHandle, RouteHandoffRequest,
    SnapshotRestoreLimits, TickInput, TrafficWorld, VehicleHandle, VehicleSpawnInput, WorldConfig,
    WorldConfigInput, WorldInstallInput, deterministic_state_digest, encode_lfrs, restore_lfrs,
};
use laneflow_spatial::{CanonicalPoseF32, FramePlacementToken, SpatialSession};
use laneflow_static_contract::RoadVehicleGeometry;
use laneflow_synth_generator::{Case, Identity, LaneChangeParams, Provenance, generate};

use crate::{handoff, install::Sample};

fn first_changer() -> (TrafficWorld, RouteHandoffRequest) {
    let identity = Identity::new(
        10_000,
        Case::LaneChange(LaneChangeParams {
            clusters: 1,
            changers_per_cluster: 2,
        }),
        33,
    )
    .unwrap();
    let bundle = generate(
        identity,
        Provenance {
            source_commit: "a".repeat(40),
            generator_commit: "b".repeat(40),
        },
    )
    .unwrap();
    let execution = ExecutionConfig::new(NonZeroU32::MIN);
    let sample = Sample::new(&bundle, execution).unwrap();
    let request = handoff::requests(
        &sample.world,
        &sample.state,
        &sample.handles,
        &sample.routes,
    )
    .unwrap()[0];
    let start = sample.world.vehicle(request.vehicle).unwrap();
    let record = &sample.state.routes[sample
        .routes
        .iter()
        .position(|&route| route == request.route)
        .unwrap()];
    let config = WorldConfig::try_new(WorldConfigInput {
        vehicle_capacity: 1,
        route_capacity: 1,
        route_edge_occurrence_capacity: record.edge_ids.len() as u64,
        route_conflict_occurrence_capacity: 16,
        fixed_delta_time_ms: sample.world.config().fixed_delta_time_ms(),
    })
    .unwrap();
    let mut world = TrafficWorld::install(WorldInstallInput {
        revision: sample.world.revision(),
        config: handoff::configure_world(&sample.state, config).unwrap(),
        execution,
        source: sample.world.committed_source().clone(),
        world_id: sample.world.world_id(),
        policy_selection: sample.world.policy_selection(),
    })
    .unwrap();
    let route = handoff::register_route(&mut world, record).unwrap();
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            start.profile(),
            route,
            start.route_edge_index(),
            start.progress_mm(),
            start.speed_mm_s(),
        ))
        .unwrap();
    (
        world,
        RouteHandoffRequest {
            route,
            vehicle,
            ..request
        },
    )
}

fn restore_host(world: &TrafficWorld) -> (App, VehicleHandle, RouteHandle) {
    let snapshot = world.capture_snapshot().unwrap();
    let [saved] = snapshot.vehicles() else {
        panic!("单车夹具")
    };
    let restored = restore_lfrs(
        &encode_lfrs(&snapshot),
        world.revision(),
        world.committed_source().clone(),
        world.config(),
        world.execution_config(),
        SnapshotRestoreLimits::new(4 << 20, 4096),
    )
    .unwrap();
    let vehicle = restored
        .vehicle_handle(saved.snapshot_vehicle_id())
        .unwrap();
    let route = restored.route_handle(saved.snapshot_route_id()).unwrap();
    let spatial = SpatialSession::bind(world.revision()).unwrap().unwrap();
    let session = LaneFlowSession::new(
        restored.into_world(),
        Some(spatial),
        LaneFlowSessionConfig::new(NonZeroU32::MIN),
    )
    .unwrap();
    let mut time = Time::<()>::default();
    time.advance_by(Duration::from_millis(world.config().fixed_delta_time_ms()));
    let mut app = App::new();
    app.add_plugins(LaneFlowPlugin)
        .insert_resource(time)
        .insert_resource(session);
    (app, vehicle, route)
}

#[derive(Debug, PartialEq)]
struct Placement {
    front: Vec3,
    forward: Vec3,
    up: Vec3,
    wheels: [Vec3; 4],
}

fn midpoint(bounds: [[f64; 2]; 3]) -> Vec3 {
    Vec3::from_array(bounds.map(|[a, b]| a.midpoint(b) as f32))
}

fn rigid_wheels(rear: Vec3, forward: Vec3, left: Vec3, geometry: RoadVehicleGeometry) -> [Vec3; 4] {
    let axle = forward * (geometry.wheelbase_mm as f32 / 1000.);
    let half_track = left * (geometry.track_width_mm as f32 / 2000.);
    [
        rear + axle + half_track,
        rear + axle - half_track,
        rear + half_track,
        rear - half_track,
    ]
}

impl Placement {
    // Bevy 的接地前杠变换朝局部 -Z，左轮在局部 -X。夹具路面水平。
    fn from_host(pose: CanonicalPoseF32, geometry: RoadVehicleGeometry, length_mm: u32) -> Self {
        let point = pose.position();
        let tangent = pose.tangent();
        let normal = pose.up();
        let front = Vec3::new(point.x(), point.y(), point.z());
        let forward = Vec3::new(tangent.x(), tangent.y(), tangent.z());
        let up = Vec3::new(normal.x(), normal.y(), normal.z());
        let transform = Transform::from_translation(front).looking_to(forward, up);
        let rear_axle = (length_mm - geometry.rear_overhang_mm) as f32 / 1000.;
        let front_axle = rear_axle - geometry.wheelbase_mm as f32 / 1000.;
        let half_track = geometry.track_width_mm as f32 / 2000.;
        let height = geometry.reference_height_mm as f32 / 1000.;
        let wheels = [
            (-half_track, front_axle),
            (half_track, front_axle),
            (-half_track, rear_axle),
            (half_track, rear_axle),
        ]
        .map(|(x, z)| transform.transform_point(Vec3::new(x, height, z)));
        Self {
            front,
            forward,
            up,
            wheels,
        }
    }

    fn assert_continuous(&self, next: &Self) {
        // 33 ms 内 3 m/s 横移首车前进约 0.1 m，保留刚体转动时各点的额外位移。
        for (p, q) in
            std::iter::once((&self.front, &next.front)).chain(self.wheels.iter().zip(&next.wheels))
        {
            assert!(
                p.distance(*q) < 0.25,
                "接地前杠或轮位跨拍跳变 {p:?} → {q:?}"
            );
        }
        assert!(self.forward.dot(next.forward) > 0.995 && self.up.dot(next.up) > 0.995);
    }
}

fn extract(
    session: &mut LaneFlowSession,
    vehicle: VehicleHandle,
    headless: &TrafficWorld,
    baseline_vehicle: VehicleHandle,
    full: &mut LaneFlowCommittedPoseBatch,
    selected: &mut LaneFlowCommittedPoseBatch,
) -> Placement {
    let tick = headless.tick_index();
    let token = FramePlacementToken::new(1);
    session.extract_committed_pose_batch(token, full).unwrap();
    session
        .extract_selected_committed_pose_batch(
            session.consumption_context(),
            &[vehicle],
            token,
            selected,
        )
        .unwrap();
    assert_eq!(full.vehicles(), &[vehicle]);
    assert_eq!(selected.vehicles(), full.vehicles());
    assert_eq!(selected.batch(), full.batch());
    assert_eq!(selected.tick(), tick);
    assert_eq!(full.tick(), tick);
    assert_eq!(selected.handoffs(), full.handoffs());

    let state = headless.vehicle(baseline_vehicle).unwrap();
    let revision = headless.revision();
    let profile = revision
        .traffic()
        .relations()
        .vehicle_profile(state.profile())
        .unwrap();
    let geometry = profile.geometry();
    let pose = full.batch().records()[0].pose();
    let placement = Placement::from_host(pose, geometry, profile.length_mm());
    let activity = headless
        .committed_handoff_geometry(baseline_vehicle)
        .unwrap();
    assert_eq!(activity.is_some(), !full.handoffs().is_empty());
    if let Some(activity) = activity {
        let [(record, presented)] = full.handoffs() else {
            panic!("单车唯一活动记录")
        };
        assert_eq!(*record, full.batch().records()[0].record());
        assert_eq!(presented.tick, tick);
        assert_eq!(presented.frame, full.batch().canonical_frame().unwrap());
        assert_eq!(
            presented.world_generation,
            session.world().world_generation()
        );
        assert_eq!(
            presented.route,
            session.world().vehicle(vehicle).unwrap().route()
        );
        assert_eq!(presented.revision, activity.revision);
        assert_eq!(presented.profile, state.profile());
        assert_eq!(presented.source_index, activity.source_index);
        assert_eq!(presented.front_occurrence, state.route_edge_index());
        assert_eq!(presented.front_handed_off, activity.front_handed_off);
        assert_eq!(presented.pose, activity.pose);
        assert_eq!(presented.footprint, activity.footprint);
        check_active_pose(&placement, &activity, geometry);
    } else {
        // L1 首车源边、目标边均沿 +Z，边起点 z=0，中央车道中心 x=0。
        let lateral = if state.route_edge_index() == 0 {
            3.5
        } else {
            0.0
        };
        let front = Vec3::new(
            lateral,
            0.,
            state.rounded_front_progress_mm() as f32 / 1000.,
        );
        assert!(
            placement.front.distance(front) <= 0.001,
            "{:?} ≠ {front:?}",
            placement.front
        );
        assert_eq!(placement.forward, Vec3::Z);
        assert_eq!(placement.up, Vec3::Y);
    }
    placement
}

fn check_active_pose(
    placement: &Placement,
    activity: &CommittedHandoffGeometry,
    geometry: RoadVehicleGeometry,
) {
    let rear = midpoint(activity.pose.rear_m);
    let forward = midpoint(activity.pose.forward).normalize();
    let left = midpoint(activity.pose.left).normalize();
    let up = midpoint(activity.pose.up).normalize();
    let front = Vec3::from_array(
        activity
            .footprint
            .front_ground_mm
            .expect("L1 前杠路面坐标应有整数毫米包络")
            .map(|[a, b]| (a as f64).midpoint(b as f64) as f32 / 1000.),
    );
    assert!(placement.front.distance(front) <= 0.001);
    assert!(placement.forward.distance(forward) <= 0.001);
    assert!(placement.up.distance(up) <= 0.001);
    for (host, traffic) in placement
        .wheels
        .iter()
        .zip(rigid_wheels(rear, forward, left, geometry))
    {
        assert!(host.distance(traffic) <= 0.001);
    }
}

fn assert_downstream_events(
    headless: &TrafficWorld,
    baseline: VehicleHandle,
    hosted: &TrafficWorld,
    owner: VehicleHandle,
) {
    use laneflow_runtime::TrafficTransitionKind::{
        ConflictEntered, GateCrossed, ReservationAcquired,
    };
    let normalized = |world: &TrafficWorld, vehicle: VehicleHandle| {
        let route = world.vehicle(vehicle).unwrap().route();
        let events = world.latest_transition_events();
        let [gate_event, grant, entry] = events else {
            panic!("预期过门、取得预约、进入冲突区")
        };
        let GateCrossed { gate } = gate_event.kind() else {
            panic!("预期过门")
        };
        let ReservationAcquired {
            passage_range: range,
        } = grant.kind()
        else {
            panic!("预期取得预约")
        };
        let ConflictEntered { passage } = entry.kind() else {
            panic!("预期进入冲突区")
        };
        assert_eq!(range.route(), route);
        assert_eq!(passage.route(), route);
        let anchors = events
            .iter()
            .map(|event| {
                assert_eq!(event.vehicle(), vehicle);
                let anchor = event.anchor();
                assert_eq!(anchor.route(), route);
                (
                    event.tick(),
                    event.vehicle_update_sequence(),
                    anchor.maneuver_occurrence_index(),
                    anchor.hop(),
                    anchor.position(),
                )
            })
            .collect::<Vec<_>>();
        (
            anchors,
            gate,
            (
                range.maneuver_occurrence_index(),
                range.admission_gate_hop(),
                range.first_conflict_occurrence_index(),
                range.passage_count(),
            ),
            (
                passage.maneuver_occurrence_index(),
                passage.admission_gate_hop(),
                passage.conflict_occurrence_index(),
                passage.address(),
                passage.stable_locator(),
            ),
        )
    };
    assert_eq!(normalized(hosted, owner), normalized(headless, baseline));
}

#[test]
fn committed_host_pose_and_wheels_stay_continuous_through_front_handoff_and_tail_clearance() {
    let (mut headless, request) = first_changer();
    let (mut app, mut owner, route) = restore_host(&headless);
    let mut full = LaneFlowCommittedPoseBatch::new();
    let mut selected = LaneFlowCommittedPoseBatch::new();
    let mut previous = extract(
        &mut app.world_mut().resource_mut::<LaneFlowSession>(),
        owner,
        &headless,
        request.vehicle,
        &mut full,
        &mut selected,
    );
    app.world_mut()
        .resource_mut::<LaneFlowSession>()
        .set_next_route_handoffs(vec![RouteHandoffRequest {
            vehicle: owner,
            route,
            ..request
        }]);
    let mut front_tick = None;
    let mut source_clear_tick = None;
    let mut release_tick = None;
    for tick in 1..=637 {
        let requests = if tick == 1 {
            std::slice::from_ref(&request)
        } else {
            &[]
        };
        let outcome = headless
            .step_with_route_handoffs(TickInput::new(33), requests)
            .unwrap();
        app.update();
        let mut session = app.world_mut().resource_mut::<LaneFlowSession>();
        assert_eq!(session.frame_step_results(), std::slice::from_ref(&outcome));
        if tick < 637 {
            assert!(headless.latest_transition_events().is_empty());
            assert!(session.world().latest_transition_events().is_empty());
        } else {
            assert_downstream_events(&headless, request.vehicle, session.world(), owner);
        }
        assert_eq!(
            deterministic_state_digest(&session.world().capture_snapshot().unwrap()).unwrap(),
            deterministic_state_digest(&headless.capture_snapshot().unwrap()).unwrap(),
            "tick={tick}"
        );
        if tick > 307 {
            continue;
        }
        let current = extract(
            &mut session,
            owner,
            &headless,
            request.vehicle,
            &mut full,
            &mut selected,
        );
        previous.assert_continuous(&current);
        let stage = headless.vehicle_route_handoffs(request.vehicle).next();
        assert_eq!(stage.is_some(), !full.handoffs().is_empty());
        if let Some((_, geometry)) = full.handoffs().first() {
            if geometry.footprint.sides[0].is_none() {
                assert!(geometry.front_handed_off);
                source_clear_tick.get_or_insert(tick);
            } else {
                assert!(source_clear_tick.is_none(), "已清空的车尾不得重新占用源道");
            }
        }
        if stage.is_some_and(|p| p.front_handed_off) && front_tick.is_none() {
            front_tick = Some(tick);
            let context = session.consumption_context();
            assert_eq!(
                session.extract_selected_committed_pose_batch(
                    context,
                    &[owner, owner],
                    FramePlacementToken::new(2),
                    &mut selected,
                ),
                Err(LaneFlowAdapterError::DuplicatePoseSelection {
                    vehicle: owner,
                    first_index: 0,
                    duplicate_index: 1,
                })
            );
            assert_eq!(selected.context(), full.context());
            assert_eq!(selected.tick(), full.tick());
            assert_eq!(selected.vehicles(), full.vehicles());
            assert_eq!(selected.batch(), full.batch());
            assert_eq!(selected.handoffs(), full.handoffs());
            (app, owner, _) = restore_host(session.world());
            let restored = extract(
                &mut app.world_mut().resource_mut::<LaneFlowSession>(),
                owner,
                &headless,
                request.vehicle,
                &mut full,
                &mut selected,
            );
            assert_eq!(
                restored, current,
                "冷恢复的宿主应在同一拍重建同一姿态与轮位"
            );
        }
        if stage.is_none() && release_tick.is_none() {
            assert!(
                source_clear_tick.is_some(),
                "释放预约前须观测到源道物理清空"
            );
            release_tick = Some(tick);
        }
        previous = current;
    }
    assert_eq!(
        (front_tick, source_clear_tick, release_tick),
        (Some(118), Some(208), Some(306))
    );
}
