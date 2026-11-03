use std::{num::NonZeroU32, sync::Arc, time::Duration};

use bevy_app::App;
use bevy_ecs::{
    resource::Resource,
    schedule::IntoScheduleConfigs,
    system::{Res, ResMut},
};
use bevy_time::{Time, TimePlugin, TimeUpdateStrategy, Virtual};
use bevy_transform::{TransformPlugin, components::Transform};
use laneflow_bevy::{
    LaneFlowConsumptionContext, LaneFlowFixed, LaneFlowFixedSet, LaneFlowSession,
    LaneFlowSessionConfig,
};
use laneflow_runtime::{
    CommittedNetworkSource, ExecutionConfig, ParkingBinding, ParkingError, ParkingTarget,
    PoseSource, PublishedLfcaReference, TrafficWorld, VehicleHandle, VehicleStatus, WorldConfig,
};
use laneflow_static_contract::LaneEdgeOrdinal;

use super::{
    parking_cli::{self, Cli},
    parking_host::{self as host, classify},
    parking_scene::{self as scene, Role, SceneConfig},
    parking_script::{Phase, Script},
};

const DELTA: Duration = Duration::from_millis(16);
const ENTRY_A_MM: u32 = 40_000;
const EXIT_A_MM: u32 = 55_000;

fn short() -> SceneConfig {
    SceneConfig {
        laps: 3,
        dwell_ticks: 60,
    }
}

fn app(config: SceneConfig) -> App {
    let mut app = App::new();
    app.add_plugins((TimePlugin, TransformPlugin));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    app.world_mut()
        .resource_mut::<Time<Virtual>>()
        .set_max_delta(Duration::from_secs(60));
    host::install(&mut app, scene::build(config).unwrap()).unwrap();
    app.update();
    healthy(&app);
    app
}

fn healthy(app: &App) {
    assert!(
        app.world().resource::<host::Health>().failure.is_none(),
        "{:?}",
        app.world().resource::<host::Health>().failure
    );
}

fn frame(app: &mut App) {
    app.insert_resource(TimeUpdateStrategy::ManualDuration(DELTA));
    app.update();
    healthy(app);
}

/// 逐帧推进到条件成立；每帧恰好一拍，超限说明脚本卡死。
fn until(app: &mut App, limit: u32, label: &str, mut done: impl FnMut(&App) -> bool) {
    for _ in 0..limit {
        if done(app) {
            return;
        }
        frame(app);
    }
    panic!("{label}：{limit} 拍内未发生");
}

fn slot(app: &App, role: Role) -> usize {
    app.world()
        .resource::<Script>()
        .actors
        .iter()
        .position(|actor| actor.role == role)
        .unwrap()
}

fn vehicle(app: &App, role: Role) -> VehicleHandle {
    app.world().resource::<Script>().actors[slot(app, role)].vehicle
}

fn phase(app: &App, role: Role) -> Phase {
    app.world().resource::<Script>().actors[slot(app, role)].phase
}

fn session(app: &App) -> &LaneFlowSession {
    app.world().resource::<LaneFlowSession>()
}

fn status(app: &App, role: Role) -> VehicleStatus {
    session(app)
        .world()
        .vehicle(vehicle(app, role))
        .unwrap()
        .status()
}

fn binding(app: &App, role: Role) -> Option<ParkingBinding> {
    session(app).world().parking_binding(vehicle(app, role))
}

fn source(app: &App, role: Role) -> Option<PoseSource> {
    session(app)
        .world()
        .committed_pose_source(vehicle(app, role))
        .unwrap()
}

fn visible(app: &App, role: Role) -> bool {
    app.world().resource::<host::Presentation>().visible[slot(app, role)]
}

fn entity(app: &App, role: Role) -> bevy_ecs::entity::Entity {
    app.world().resource::<host::Fleet>().entities[slot(app, role)]
}

fn transform(app: &App, role: Role) -> Transform {
    *app.world().get::<Transform>(entity(app, role)).unwrap()
}

fn in_batch(app: &App, role: Role) -> bool {
    app.world()
        .resource::<host::Presentation>()
        .poses
        .vehicles()
        .contains(&vehicle(app, role))
}

fn counts(app: &App) -> laneflow_runtime::ParkingFacilityCounts {
    let facility = app.world().resource::<host::Fleet>().facility;
    session(app)
        .world()
        .parking_facility_counts(facility)
        .unwrap()
}

fn parked_virtual(app: &App) -> bool {
    status(app, Role::Virtual) == VehicleStatus::Parked
}

#[test]
fn p01_cli_accepts_bounds_and_rejects_invalid_arguments() {
    let parse = |args: &[&str]| parking_cli::parse(args.iter().map(|arg| (*arg).to_owned()));
    assert_eq!(parse(&[]).unwrap(), Cli::Run { capture: None });
    assert_eq!(parse(&["--help"]).unwrap(), Cli::Help);
    assert!(parking_cli::HELP.contains("--at-tick"));
    assert!(matches!(
        parse(&["--screenshot", "a.png", "--at-tick", "50000"]).unwrap(),
        Cli::Run {
            capture: Some(request)
        } if request.tick == 50_000
    ));
    for invalid in [
        &["--at-tick", "1"][..],
        &["--screenshot", "a.jpg"],
        &["--screenshot"],
        &["--screenshot", "a.png", "--at-tick", "50001"],
        &["--screenshot", "a.png", "--at-tick", "-1"],
        &["--cells", "1"],
        &["--help", "--help"],
    ] {
        assert!(parse(invalid).is_err(), "{invalid:?}");
    }
}

#[test]
fn p01_startup_binds_every_vehicle_and_repeats_the_same_plan() {
    let first = app(short());
    let second = app(short());
    let world = first.world();
    let actors = &world.resource::<Script>().actors;
    assert_eq!(actors.len(), 6);
    for (slot, actor) in actors.iter().enumerate() {
        assert_eq!(
            session(&first).vehicle_entity(actor.vehicle),
            Some(world.resource::<host::Fleet>().entities[slot])
        );
        assert_eq!(
            actor.vehicle,
            second.world().resource::<Script>().actors[slot].vehicle
        );
    }
    for role in [Role::Explicit, Role::Virtual, Role::Blocker, Role::Removal] {
        assert_eq!(transform(&first, role), transform(&second, role));
        assert!(visible(&first, role));
    }
    let layout = scene::build(short()).unwrap();
    assert_eq!(layout.lanes.len(), 4);
    assert_eq!(layout.pads.len(), 2);
    assert!(layout.max.x - layout.min.x >= 120.0);
    assert!(
        layout
            .pads
            .iter()
            .all(|pad| pad.length > layout.vehicle_length)
    );
    assert!(layout.pads.iter().all(|pad| pad.width > 0.0));
    assert!(layout.lane_width > 0.0);
    assert_ne!(layout.pads[0].space, layout.pads[1].space);
    assert_eq!(
        world.resource::<host::Fleet>().vehicle_length,
        layout.vehicle_length
    );
    host::transform_from_pose(layout.garage_entry).unwrap();
    host::transform_from_pose(layout.garage_exit).unwrap();
}

#[test]
fn p02_explicit_vehicle_parks_on_its_space_and_returns_to_the_lane_with_the_same_entity() {
    let mut app = app(short());
    let body = entity(&app, Role::Explicit);
    let pad = scene::build(short()).unwrap().pads[0].pose;
    until(&mut app, 2_000, "显式车到达", |app| {
        let binding = binding(app, Role::Explicit);
        binding.is_some_and(|binding| {
            session(app)
                .world()
                .parking_arrived(vehicle(app, Role::Explicit), binding.target())
        })
    });
    let state = session(&app)
        .world()
        .vehicle(vehicle(&app, Role::Explicit))
        .unwrap();
    assert_eq!(state.progress_mm(), ENTRY_A_MM);
    assert_eq!(state.speed_mm_s(), 0);
    assert!(visible(&app, Role::Explicit));
    frame(&mut app);
    assert_eq!(status(&app, Role::Explicit), VehicleStatus::Parked);
    assert!(matches!(
        source(&app, Role::Explicit),
        Some(PoseSource::Parking { .. })
    ));
    assert!(visible(&app, Role::Explicit));
    assert_eq!(
        transform(&app, Role::Explicit),
        host::transform_from_pose(pad).unwrap()
    );
    assert_eq!(
        session(&app).vehicle_entity(vehicle(&app, Role::Explicit)),
        Some(body)
    );
    until(&mut app, 2_000, "显式车离场", |app| {
        status(app, Role::Explicit) == VehicleStatus::Active
    });
    let state = session(&app)
        .world()
        .vehicle(vehicle(&app, Role::Explicit))
        .unwrap();
    assert!(state.progress_mm() >= EXIT_A_MM);
    assert_eq!(state.route_edge_index(), 0);
    assert!(matches!(
        source(&app, Role::Explicit),
        Some(PoseSource::Lane { .. })
    ));
    assert!(visible(&app, Role::Explicit));
    assert_eq!(
        session(&app).vehicle_entity(vehicle(&app, Role::Explicit)),
        Some(body)
    );
}

#[test]
fn p03_parked_vehicle_releases_the_lane_for_following_traffic() {
    let mut app = app(SceneConfig {
        laps: 3,
        dwell_ticks: 600,
    });
    until(&mut app, 2_000, "显式车停入", |app| {
        status(app, Role::Explicit) == VehicleStatus::Parked
    });
    let through = vehicle(&app, Role::Through);
    let start = session(&app).world().vehicle(through).unwrap();
    assert_eq!(start.route_edge_index(), 0);
    assert!(start.progress_mm() < ENTRY_A_MM);
    until(&mut app, 500, "过路车通过", |app| {
        session(app).world().vehicle(through).unwrap().progress_mm() > ENTRY_A_MM + 5_000
    });
    assert_eq!(status(&app, Role::Explicit), VehicleStatus::Parked);
}

#[test]
fn p04_p08_virtual_parking_hides_without_touching_identity_or_transform() {
    let mut app = app(short());
    let body = entity(&app, Role::Virtual);
    let handle = vehicle(&app, Role::Virtual);
    until(&mut app, 2_000, "虚拟车停入", parked_virtual);
    assert_eq!(
        binding(&app, Role::Virtual),
        Some(ParkingBinding::Occupied(ParkingTarget::VirtualPool(
            app.world().resource::<host::Fleet>().facility
        )))
    );
    assert!(counts(&app).virtual_pool.occupied >= 1);
    let frozen = transform(&app, Role::Virtual);
    // 车辆已隐藏，连续多帧缺席批次也不等于移除。
    for _ in 0..40 {
        assert!(parked_virtual(&app));
        assert!(source(&app, Role::Virtual).is_none());
        assert!(!in_batch(&app, Role::Virtual));
        assert!(!visible(&app, Role::Virtual));
        assert_eq!(session(&app).vehicle_entity(handle), Some(body));
        assert_eq!(transform(&app, Role::Virtual), frozen);
        frame(&mut app);
    }
}

#[test]
fn p05_early_park_is_rejected_and_changes_nothing() {
    let mut app = app(short());
    until(&mut app, 20, "过早停入", |app| {
        app.world().resource::<Script>().last_rejection.is_some()
    });
    let rejection = app.world().resource::<Script>().last_rejection.unwrap();
    assert_eq!(rejection.role, Role::Virtual);
    assert_eq!(rejection.command, "park");
    assert_eq!(rejection.error, ParkingError::NotArrived);
    assert_eq!(status(&app, Role::Virtual), VehicleStatus::Active);
    assert!(matches!(
        binding(&app, Role::Virtual),
        Some(ParkingBinding::Reserved(_))
    ));
    assert!(matches!(
        source(&app, Role::Virtual),
        Some(PoseSource::Lane { .. })
    ));
    assert!(visible(&app, Role::Virtual));
}

#[test]
fn p06_p07_blocked_leave_stays_hidden_then_retries_and_reappears_in_the_same_frame() {
    let mut app = app(short());
    let body = entity(&app, Role::Virtual);
    until(&mut app, 2_000, "虚拟离场被拒", |app| {
        app.world()
            .resource::<Script>()
            .last_rejection
            .is_some_and(|rejection| rejection.command == "leave")
    });
    let rejection = app.world().resource::<Script>().last_rejection.unwrap();
    assert_eq!(rejection.role, Role::Virtual);
    assert_eq!(
        rejection.error,
        ParkingError::LeavePhysicalOverlap {
            blocker: vehicle(&app, Role::Blocker)
        }
    );
    assert!(parked_virtual(&app));
    assert!(!visible(&app, Role::Virtual));
    // 阻挡车在同一拍收到放行并停入，虚拟车下一拍重试。
    assert_eq!(status(&app, Role::Blocker), VehicleStatus::Parked);
    let hidden_at = transform(&app, Role::Virtual);
    until(&mut app, 200, "虚拟车重试成功", |app| {
        if parked_virtual(app) {
            assert!(!visible(app, Role::Virtual));
            assert_eq!(transform(app, Role::Virtual), hidden_at);
        }
        !parked_virtual(app)
    });
    // 离场成功的这一帧：同一 Entity 已可见，Transform 就是本批位姿。
    assert!(visible(&app, Role::Virtual));
    assert_eq!(
        session(&app).vehicle_entity(vehicle(&app, Role::Virtual)),
        Some(body)
    );
    let presentation = app.world().resource::<host::Presentation>();
    let row = presentation
        .poses
        .vehicles()
        .iter()
        .position(|candidate| *candidate == vehicle(&app, Role::Virtual))
        .unwrap();
    let pose = presentation.poses.batch().records()[row].pose();
    assert_eq!(
        transform(&app, Role::Virtual),
        host::transform_from_pose(pose).unwrap()
    );
    assert_ne!(transform(&app, Role::Virtual), hidden_at);
}

#[test]
fn p09_only_typed_removal_clears_the_mapping_and_destroys_the_entity() {
    let mut app = app(short());
    let handle = vehicle(&app, Role::Removal);
    let body = entity(&app, Role::Removal);
    until(&mut app, 2_000, "移除车停入", |app| {
        status(app, Role::Removal) == VehicleStatus::Parked
    });
    let before = counts(&app).virtual_pool;
    // 停留期间车辆缺席批次，映射和实体都还在。
    assert_eq!(session(&app).vehicle_entity(handle), Some(body));
    assert!(app.world().get_entity(body).is_ok());
    until(&mut app, 200, "移除", |app| {
        phase(app, Role::Removal) == Phase::Removed
    });
    assert_eq!(session(&app).vehicle_entity(handle), None);
    assert!(session(&app).world().vehicle(handle).is_none());
    assert!(app.world().get_entity(body).is_err());
    assert!(!visible(&app, Role::Removal));
    // 虚拟车可能在同一窗口离场，只断言移除车释放了它占用的容量。
    assert!(counts(&app).virtual_pool.occupied < before.occupied);
    for role in [Role::Explicit, Role::Virtual, Role::Blocker, Role::Through] {
        assert!(session(&app).vehicle_entity(vehicle(&app, role)).is_some());
    }
    for _ in 0..30 {
        frame(&mut app);
    }
    let summary = app
        .world()
        .resource::<host::Summary>()
        .current
        .clone()
        .unwrap();
    assert_eq!(summary.rows[slot(&app, Role::Removal)].stage, "removed");
}

fn finish(app: &mut App, limit: u32) {
    until(app, limit, "全部车辆走完", |app| {
        app.world().resource::<Script>().actors.iter().all(|actor| {
            actor.phase == Phase::Removed
                || session(app)
                    .world()
                    .vehicle(actor.vehicle)
                    .unwrap()
                    .status()
                    == VehicleStatus::Completed
        })
    });
    let world = app.world();
    for (slot, actor) in world.resource::<Script>().actors.iter().enumerate() {
        assert!(!world.resource::<host::Presentation>().visible[slot]);
        if actor.phase != Phase::Removed {
            assert_eq!(actor.phase, Phase::Drive);
            assert_eq!(session(app).world().parking_binding(actor.vehicle), None);
            assert!(session(app).vehicle_entity(actor.vehicle).is_some());
        }
    }
}

#[test]
fn p10_finished_routes_stop_reserving_and_hide_with_the_mapping_kept() {
    let mut app = app(short());
    finish(&mut app, 30_000);
}

#[test]
fn p10_longer_run_with_default_dwell_does_not_deadlock() {
    let mut app = app(SceneConfig {
        laps: 6,
        dwell_ticks: SceneConfig::new().dwell_ticks,
    });
    finish(&mut app, 60_000);
}

#[test]
fn p11_classification_rejects_every_combination_outside_the_table() {
    let space = laneflow_static_contract::ParkingSpaceOrdinal::from_raw(0);
    let other = laneflow_static_contract::ParkingSpaceOrdinal::from_raw(1);
    let facility = laneflow_static_contract::ParkingFacilityOrdinal::from_raw(0);
    let lane = Some(PoseSource::Lane {
        edge: LaneEdgeOrdinal::from_raw(0),
        progress_mm: 0,
    });
    let parking = Some(PoseSource::Parking { space });
    let explicit = Some(ParkingBinding::Occupied(ParkingTarget::ExplicitSpace(
        space,
    )));
    let pool = Some(ParkingBinding::Occupied(ParkingTarget::VirtualPool(
        facility,
    )));
    let elsewhere = Some(ParkingBinding::Occupied(ParkingTarget::ExplicitSpace(
        other,
    )));
    use VehicleStatus::{Active, Completed, Parked};
    for (status, binding, source, in_batch, expected) in [
        (Active, None, lane, true, Ok(true)),
        (Parked, explicit, parking, true, Ok(true)),
        (Parked, pool, None, false, Ok(false)),
        (Completed, None, None, false, Ok(false)),
    ] {
        assert_eq!(
            classify(status, binding, source, in_batch).map_err(|_| ()),
            expected.map_err(|_: ()| ())
        );
    }
    for (status, binding, source, in_batch) in [
        // 缺席不能解释成隐藏。
        (Active, None, None, false),
        (Active, None, lane, false),
        (Parked, explicit, parking, false),
        (Parked, pool, None, true),
        (Parked, pool, parking, true),
        (Parked, explicit, None, false),
        (Parked, elsewhere, parking, true),
        (Parked, None, None, false),
        (Active, explicit, lane, true),
        (Completed, None, None, true),
        (Completed, pool, None, false),
    ] {
        assert!(
            classify(status, binding, source, in_batch).is_err(),
            "{status:?} {binding:?} {source:?} {in_batch}"
        );
    }
}

fn committed_view(app: &App) -> (Vec<bool>, Vec<Transform>) {
    let world = app.world();
    let fleet = world.resource::<host::Fleet>();
    (
        world.resource::<host::Presentation>().visible.clone(),
        fleet
            .entities
            .iter()
            .map(|entity| *world.get::<Transform>(*entity).unwrap())
            .collect(),
    )
}

fn assert_rejected(mut app: App, before: &(Vec<bool>, Vec<Transform>), expected: &str) {
    assert_eq!(&committed_view(&app), before);
    let failure = app.world().resource::<host::Health>();
    let detail = &failure.failure.as_ref().expect("必须记录失败").detail;
    assert!(detail.contains(expected), "{detail}");
    assert!(app.world().resource::<host::Summary>().current.is_none());
    host::exit_on_failure(app.world_mut());
    assert!(app.should_exit().is_some_and(|exit| exit.is_error()));
}

/// 另一世界的消费上下文：同修订，只有 `world_id` 不同。
fn foreign_context(app: &App) -> LaneFlowConsumptionContext {
    let session = session(app);
    let revision = session.world().revision();
    let origin = revision.canonical_origin();
    let world = TrafficWorld::install(
        Arc::clone(&revision),
        WorldConfig::new(1, 1, 1_024, 1_024, 16),
        ExecutionConfig::new(NonZeroU32::MIN),
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "scenario://parking-foreign",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .unwrap(),
        },
        931,
        session.world().policy_selection(),
    )
    .unwrap();
    LaneFlowSession::new(world, None, LaneFlowSessionConfig::new(NonZeroU32::MIN))
        .unwrap()
        .consumption_context()
}

#[test]
fn p12_stale_context_rejects_the_whole_batch() {
    let mut app = app(short());
    until(&mut app, 2_000, "虚拟车停入", parked_virtual);
    let before = committed_view(&app);
    let context = foreign_context(&app);
    host::present_with(app.world_mut(), context);
    assert_rejected(app, &before, "过期消费上下文");
}

#[test]
fn p12_stale_placement_token_rejects_the_whole_batch_after_the_world_moved() {
    let mut app = app(short());
    until(&mut app, 200, "车辆起步", |app| {
        session(app).world().tick_index() > 100
    });
    let before = committed_view(&app);
    let root = app.world().resource::<host::Fleet>().root;
    app.world_mut().get_mut::<host::Placement>(root).unwrap().0 =
        laneflow_spatial::FramePlacementToken::new(99);
    app.insert_resource(TimeUpdateStrategy::ManualDuration(DELTA));
    app.update();
    assert_rejected(app, &before, "placement token");
}

type TraceRow = Option<(VehicleStatus, u32, Option<ParkingBinding>)>;
type TraceTick = (u64, Vec<TraceRow>);

#[derive(Resource, Default)]
struct Trace(Vec<TraceTick>);

fn record_trace(session: Res<LaneFlowSession>, script: Res<Script>, mut trace: ResMut<Trace>) {
    let rows = script
        .actors
        .iter()
        .map(|actor| {
            session.world().vehicle(actor.vehicle).map(|state| {
                (
                    state.status(),
                    state.progress_mm(),
                    session.world().parking_binding(actor.vehicle),
                )
            })
        })
        .collect();
    trace.0.push((session.world().tick_index(), rows));
}

fn trace_at(delta: Duration, ticks: usize) -> Vec<TraceTick> {
    let mut app = app(short());
    app.init_resource::<Trace>().add_systems(
        LaneFlowFixed,
        record_trace.in_set(LaneFlowFixedSet::Observe),
    );
    app.insert_resource(TimeUpdateStrategy::ManualDuration(delta));
    while app.world().resource::<Trace>().0.len() < ticks {
        app.update();
        healthy(&app);
    }
    let mut trace = app.world_mut().remove_resource::<Trace>().unwrap().0;
    trace.truncate(ticks);
    trace
}

#[test]
fn p13_script_results_do_not_depend_on_the_frame_length() {
    let ticks = 2_500;
    let baseline = trace_at(DELTA, ticks);
    assert_eq!(baseline.len(), ticks);
    assert!(baseline.iter().any(|(_, rows)| {
        rows.iter()
            .any(|row| row.is_some_and(|(status, ..)| status == VehicleStatus::Parked))
    }));
    assert_eq!(baseline, trace_at(Duration::from_millis(33), ticks));
    assert_eq!(baseline, trace_at(Duration::from_millis(64), ticks));
}

#[test]
fn p14_hud_names_the_stage_of_a_hidden_vehicle_and_the_rejected_command() {
    let mut app = app(short());
    let hud = |app: &App| {
        app.world()
            .resource::<host::Summary>()
            .current
            .as_ref()
            .unwrap()
            .hud()
    };
    assert!(hud(&app).contains("last rejected: none"));
    until(&mut app, 2_000, "虚拟车停入", parked_virtual);
    let text = hud(&app);
    assert!(text.contains("parked (virtual)"), "{text}");
    assert!(text.contains("hidden"), "{text}");
    assert!(
        text.contains("last rejected: virtual park: not arrived"),
        "{text}"
    );
    until(&mut app, 2_000, "虚拟离场被拒", |app| {
        app.world()
            .resource::<Script>()
            .last_rejection
            .is_some_and(|rejection| rejection.command == "leave")
    });
    let text = hud(&app);
    assert!(
        text.contains("last rejected: virtual leave: physical overlap"),
        "{text}"
    );
    until(&mut app, 2_000, "移除", |app| {
        phase(app, Role::Removal) == Phase::Removed
    });
    assert!(hud(&app).contains("removed"));
}
