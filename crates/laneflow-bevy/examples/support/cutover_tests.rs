use std::{sync::Arc, time::Duration};

use bevy_app::App;
use bevy_ecs::entity::Entity;
use bevy_time::{Time, TimePlugin, TimeUpdateStrategy, Virtual};
use bevy_transform::{TransformPlugin, components::Transform};
use laneflow_bevy::{LaneFlowAdapterError, LaneFlowCommittedPoseBatch, LaneFlowSession};
use laneflow_runtime::{ParkingBinding, ParkingTarget, PoseSource, VehicleHandle, VehicleStatus};
use laneflow_spatial::{
    CanonicalPoseBatch, FramePlacementToken, PoseInput, PoseRecordId, SpatialSession,
};
use laneflow_static_contract::{
    LaneEdgeOrdinal, NetworkRevisionId, ParkingFacilityOrdinal, ParkingSpaceOrdinal,
};
use laneflow_static_network::SharedNetworkRevision;

use super::{
    cutover_cli::{self, Cli},
    cutover_host::{
        self as host, Cutover, CutoverState, Fleet, Health, Presentation, RefusalReason, RoadData,
        SPUR_SLOT, Summary, classify,
    },
    cutover_scene::{self as scene, RevisionName},
};

const DELTA: Duration = Duration::from_millis(16);
const R1_NORTH_MM: u32 = 120_000;

fn app_with(plan: Vec<u64>) -> App {
    let mut app = App::new();
    app.add_plugins((TimePlugin, TransformPlugin));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    app.world_mut()
        .resource_mut::<Time<Virtual>>()
        .set_max_delta(Duration::from_secs(60));
    host::install(&mut app, scene::build().unwrap(), plan).unwrap();
    app.update();
    healthy(&app);
    app
}

fn app() -> App {
    app_with(Vec::new())
}

fn healthy(app: &App) {
    let failure = &app.world().resource::<Health>().failure;
    assert!(failure.is_none(), "{failure:?}");
}

fn frame(app: &mut App) {
    app.insert_resource(TimeUpdateStrategy::ManualDuration(DELTA));
    app.update();
    healthy(app);
}

fn frames(app: &mut App, count: u32) {
    for _ in 0..count {
        frame(app);
    }
}

fn zero_frame(app: &mut App) {
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    app.update();
    healthy(app);
}

/// 逐帧推进到条件成立；每帧恰好一拍。
fn until(app: &mut App, limit: u32, label: &str, mut done: impl FnMut(&App) -> bool) {
    for _ in 0..limit {
        if done(app) {
            return;
        }
        frame(app);
    }
    panic!("{label}：{limit} 拍内未发生");
}

fn session(app: &App) -> &LaneFlowSession {
    app.world().resource::<LaneFlowSession>()
}

fn cutover(app: &App) -> &Cutover {
    app.world().resource::<Cutover>()
}

fn fleet(app: &App) -> &Fleet {
    app.world().resource::<Fleet>()
}

fn request(app: &mut App) {
    assert!(app.world_mut().resource_mut::<Cutover>().request());
}

fn tick(app: &App) -> u64 {
    session(app).world().tick_index()
}

fn generation(app: &App) -> u64 {
    session(app).world().world_generation().get()
}

fn world_revision(app: &App) -> NetworkRevisionId {
    session(app)
        .world()
        .revision()
        .canonical_origin()
        .network_revision()
}

fn root(app: &App, name: RevisionName) -> Arc<SharedNetworkRevision> {
    Arc::clone(&cutover(app).target(name).root)
}

fn revision_id(app: &App, name: RevisionName) -> NetworkRevisionId {
    root(app, name).canonical_origin().network_revision()
}

fn ring(app: &App) -> Vec<VehicleHandle> {
    fleet(app).vehicles[..SPUR_SLOT]
        .iter()
        .map(|vehicle| vehicle.unwrap())
        .collect()
}

fn entities(app: &App) -> Vec<Entity> {
    fleet(app).entities.clone()
}

fn transforms(app: &App) -> Vec<Transform> {
    entities(app)
        .iter()
        .map(|&entity| *app.world().get::<Transform>(entity).unwrap())
        .collect()
}

fn source(app: &App, vehicle: VehicleHandle) -> Option<PoseSource> {
    session(app).world().committed_pose_source(vehicle).unwrap()
}

fn lane_source(app: &App, vehicle: VehicleHandle) -> (LaneEdgeOrdinal, u32) {
    match source(app, vehicle) {
        Some(PoseSource::Lane { edge, progress_mm }) => (edge, progress_mm),
        other => panic!("{other:?}"),
    }
}

/// 不经过宿主，直接用独立的 Spatial 在指定修订上采样。
fn sampled(
    revision: &Arc<SharedNetworkRevision>,
    edge: LaneEdgeOrdinal,
    progress_mm: u32,
) -> Transform {
    let mut spatial = SpatialSession::bind(Arc::clone(revision)).unwrap().unwrap();
    let mut batch = CanonicalPoseBatch::new();
    spatial
        .extract_pose_batch(
            FramePlacementToken::new(1),
            &[PoseInput::lane(PoseRecordId::new(0), edge, progress_mm)],
            &mut batch,
        )
        .unwrap();
    host::transform_from_pose(batch.records()[0].pose()).unwrap()
}

/// 零步进帧切到 `R2`。
fn enter_r2(app: &mut App) {
    assert_eq!(cutover(app).current, RevisionName::R1);
    request(app);
    zero_frame(app);
    assert_eq!(cutover(app).current, RevisionName::R2);
}

fn exit_code(app: &App) -> bool {
    app.should_exit().is_some_and(|exit| exit.is_error())
}

#[test]
fn r01_startup_binds_the_ring_and_prepares_both_diffs() {
    let app = app();
    let cars = ring(&app);
    assert_eq!(cars.len(), 4);
    for (&vehicle, &entity) in cars.iter().zip(&entities(&app)) {
        assert_eq!(session(&app).vehicle_entity(vehicle), Some(entity));
    }
    assert_eq!(fleet(&app).vehicles[SPUR_SLOT], None);
    assert_eq!(fleet(&app).spur_route, None);
    let cutover = cutover(&app);
    let (r1, r2) = (
        cutover.target(RevisionName::R1),
        cutover.target(RevisionName::R2),
    );
    assert!(!r1.diff.is_empty() && !r2.diff.is_empty());
    assert_ne!(r1.diff, r2.diff);
    assert_ne!(
        r1.root.canonical_origin().network_revision(),
        r2.root.canonical_origin().network_revision()
    );
    assert_eq!(world_revision(&app), revision_id(&app, RevisionName::R1));
    assert_eq!(
        app.world().resource::<RoadData>().revision,
        world_revision(&app)
    );
    assert_eq!(
        app.world().resource::<Presentation>().visible,
        [true, true, true, true, false]
    );
    assert!(fleet(&app).vehicle_length > 0.0);
    assert!(app.world().resource::<RoadData>().lane_width > 0.0);
    // 初始视角依据的范围必须同时覆盖环路和只在 `R2` 存在的支路。
    let bounds = scene::build().unwrap();
    assert_eq!((bounds.min.x, bounds.max.x), (0.0, 180.0));
    assert_eq!((bounds.min.z, bounds.max.z), (-140.0, 0.0));
}

#[test]
fn r01_command_line_is_validated() {
    let parse = |args: &[&str]| cutover_cli::parse(args.iter().map(|arg| (*arg).to_owned()));
    assert_eq!(parse(&["--help"]), Ok(Cli::Help));
    assert!(cutover_cli::HELP.contains("--cutover-at-tick"));
    assert_eq!(
        parse(&[]),
        Ok(Cli::Run {
            cutovers: Vec::new(),
            capture: None
        })
    );
    let Ok(Cli::Run { cutovers, capture }) = parse(&[
        "--cutover-at-tick",
        "900",
        "--cutover-at-tick",
        "300",
        "--screenshot",
        "out.png",
        "--at-tick",
        "1000",
    ]) else {
        panic!("合法参数被拒绝");
    };
    assert_eq!(cutovers, [300, 900]);
    assert_eq!(capture.unwrap().tick, 1_000);
    for args in [
        &["--unknown"][..],
        &["--at-tick", "10"],
        &["--screenshot", "out.jpg"],
        &["--cutover-at-tick"],
        &["--cutover-at-tick", "-1"],
        &["--cutover-at-tick", "99999999"],
        &["--cutover-at-tick", "5", "--cutover-at-tick", "5"],
    ] {
        assert!(parse(args).is_err(), "{args:?}");
    }
}

#[test]
fn r02_cutover_keeps_identity_and_updates_world_and_road_in_one_frame() {
    let mut app = app();
    frames(&mut app, 50);
    let cars = ring(&app);
    let bound = entities(&app);
    let routes: Vec<_> = cars
        .iter()
        .map(|&vehicle| session(&app).world().vehicle(vehicle).unwrap().route())
        .collect();
    let progress: Vec<_> = cars.iter().map(|&car| lane_source(&app, car).1).collect();
    let before = generation(&app);
    enter_r2(&mut app);
    let target = revision_id(&app, RevisionName::R2);
    assert_eq!(world_revision(&app), target);
    assert_eq!(app.world().resource::<RoadData>().revision, target);
    assert_eq!(app.world().resource::<RoadData>().version, 1);
    let lanes = &app.world().resource::<RoadData>().lanes;
    assert_eq!(
        *lanes,
        scene::centerlines(&root(&app, RevisionName::R2)).unwrap()
    );
    assert_eq!(lanes.len(), 5);
    assert_eq!(generation(&app), before + 1);
    assert_eq!(ring(&app), cars);
    for (index, &vehicle) in cars.iter().enumerate() {
        let state = session(&app).world().vehicle(vehicle).unwrap();
        assert_eq!(state.route(), routes[index]);
        assert_eq!(state.progress_mm(), progress[index]);
        assert_eq!(session(&app).vehicle_entity(vehicle), Some(bound[index]));
    }
    assert_eq!(cutover(&app).events, 1);
    let summary = app.world().resource::<Summary>().current.clone().unwrap();
    assert_eq!(summary.revision, RevisionName::R2);
    assert_eq!(summary.revision_id, target);
    assert_eq!(summary.generation, before + 1);
    assert_eq!(summary.events, 1);
    assert_eq!(summary.state, CutoverState::Idle);
    assert_eq!(summary.target, None);
}

#[test]
fn r03_transforms_follow_target_geometry_only_on_the_bent_edge() {
    let mut app = app();
    let before = transforms(&app);
    let cars = ring(&app);
    let target = root(&app, RevisionName::R2);
    let north = scene::edge(&target, "north").unwrap();
    enter_r2(&mut app);
    let after = transforms(&app);
    let mut bent = 0;
    for (index, &car) in cars.iter().enumerate() {
        let (edge, progress) = lane_source(&app, car);
        assert_eq!(after[index], sampled(&target, edge, progress));
        if edge == north {
            bent += 1;
            assert_ne!(after[index].translation, before[index].translation);
        } else {
            assert_eq!(after[index], before[index]);
        }
    }
    assert_eq!(bent, 1);
}

#[test]
fn r04_old_batch_and_context_are_rejected_after_cutover() {
    let mut app = app();
    frames(&mut app, 5);
    let old = app.world().resource::<Presentation>().poses.context();
    assert!(session(&app).consumption_context_is_current(old));
    let target = root(&app, RevisionName::R2);
    let car = ring(&app)[2];
    enter_r2(&mut app);
    assert!(!session(&app).consumption_context_is_current(old));
    assert_eq!(cutover(&app).stale_rejected, 1);
    // 刚切换的这一帧写入的是目标修订的采样，而不是切换前持有的批次。
    let (edge, progress) = lane_source(&app, car);
    assert_eq!(transforms(&app)[2], sampled(&target, edge, progress));
    let mut output = LaneFlowCommittedPoseBatch::new();
    let selected = app
        .world_mut()
        .resource_mut::<LaneFlowSession>()
        .extract_selected_committed_pose_batch(
            old,
            &[car],
            FramePlacementToken::new(1),
            &mut output,
        );
    assert_eq!(
        selected,
        Err(LaneFlowAdapterError::StalePoseSelectionContext)
    );
}

#[test]
fn r05_retired_spatial_is_taken_and_extraction_uses_the_target() {
    let mut app = app();
    frames(&mut app, 5);
    enter_r2(&mut app);
    assert_eq!(cutover(&app).retired, 1);
    let target = revision_id(&app, RevisionName::R2);
    let poses = &app.world().resource::<Presentation>().poses;
    assert_eq!(poses.batch().network_revision(), Some(target));
    frame(&mut app);
    let poses = &app.world().resource::<Presentation>().poses;
    assert_eq!(poses.batch().network_revision(), Some(target));
}

#[test]
fn r06_spur_car_binds_to_the_proxy_and_stays_mapped_when_completed() {
    let mut app = app();
    let proxy = entities(&app)[SPUR_SLOT];
    enter_r2(&mut app);
    let car = fleet(&app).vehicles[SPUR_SLOT].unwrap();
    assert!(fleet(&app).spur_route.is_some());
    assert_eq!(session(&app).vehicle_entity(car), Some(proxy));
    assert!(app.world().resource::<Presentation>().visible[SPUR_SLOT]);
    let target = root(&app, RevisionName::R2);
    let spur = scene::edge(&target, "spur").unwrap();
    assert_eq!(lane_source(&app, car), (spur, scene::SPUR_START_MM));
    frames(&mut app, 60);
    let (edge, progress) = lane_source(&app, car);
    assert_eq!(edge, spur);
    assert!(progress > scene::SPUR_START_MM);
    assert_eq!(
        app.world()
            .resource::<Summary>()
            .current
            .as_ref()
            .unwrap()
            .spur,
        "driving"
    );
    until(&mut app, 2_000, "支路车走完支路", |app| {
        session(app).world().vehicle(car).unwrap().status() == VehicleStatus::Completed
    });
    frame(&mut app);
    assert!(!app.world().resource::<Presentation>().visible[SPUR_SLOT]);
    assert_eq!(session(&app).vehicle_entity(car), Some(proxy));
    assert_eq!(source(&app, car), None);
    assert_eq!(
        app.world()
            .resource::<Summary>()
            .current
            .as_ref()
            .unwrap()
            .spur,
        "completed"
    );
    assert_eq!(
        app.world().resource::<Presentation>().visible[..SPUR_SLOT],
        [true; 4]
    );
}

#[test]
fn r07_r08_back_cutover_is_refused_then_succeeds_after_clearing() {
    let mut app = app();
    enter_r2(&mut app);
    frames(&mut app, 30);
    let proxy = entities(&app)[SPUR_SLOT];
    let car = fleet(&app).vehicles[SPUR_SLOT].unwrap();
    let (before_generation, before_tick) = (generation(&app), tick(&app));
    let context = session(&app).consumption_context();
    let road_version = app.world().resource::<RoadData>().version;
    request(&mut app);
    frame(&mut app);
    // R07：修订、世代、路面和上下文都没变，世界继续推进。
    let refused = cutover(&app);
    assert_eq!(refused.current, RevisionName::R2);
    assert_eq!(refused.pending, Some(RevisionName::R1));
    assert_eq!(refused.state, CutoverState::AwaitingClear);
    assert_eq!(refused.successes, 1);
    let refusal = refused.last_refusal.unwrap();
    assert_eq!(refusal.reason, RefusalReason::UnmappableLaneEdge);
    assert_eq!(refusal.tick, before_tick);
    assert_eq!(generation(&app), before_generation);
    assert_eq!(world_revision(&app), revision_id(&app, RevisionName::R2));
    assert_eq!(app.world().resource::<RoadData>().version, road_version);
    assert!(session(&app).consumption_context_is_current(context));
    assert_eq!(tick(&app), before_tick + 1);
    let hud = app
        .world()
        .resource::<Summary>()
        .current
        .as_ref()
        .unwrap()
        .hud();
    assert!(
        hud.contains(&format!(
            "cutover refused: unmappable lane edge @ tick {before_tick}"
        )),
        "{hud}"
    );
    assert!(hud.contains("waiting for clear -> R1"), "{hud}");
    // R08：同一帧已清场，下一帧重试成功。
    assert_eq!(fleet(&app).vehicles[SPUR_SLOT], None);
    assert_eq!(fleet(&app).spur_route, None);
    assert_eq!(session(&app).vehicle_entity(car), None);
    assert!(!app.world().resource::<Presentation>().visible[SPUR_SLOT]);
    frame(&mut app);
    assert_eq!(cutover(&app).current, RevisionName::R1);
    assert_eq!(cutover(&app).pending, None);
    assert_eq!(cutover(&app).successes, 2);
    assert_eq!(world_revision(&app), revision_id(&app, RevisionName::R1));
    assert!(app.world().get_entity(proxy).is_ok());
    assert!(!app.world().resource::<Presentation>().visible[SPUR_SLOT]);
}

/// 让环路车越过 `R1` 的 `north` 长度；此时回切必须等待，不能动车辆。
#[test]
fn r09_vehicle_past_target_length_defers_the_cutover_until_it_drives_off() {
    let mut app = app();
    enter_r2(&mut app);
    host::clear_spur(app.world_mut()).unwrap();
    let target = root(&app, RevisionName::R2);
    let north = scene::edge(&target, "north").unwrap();
    let cars = ring(&app);
    until(&mut app, 3_000, "环路车越过 120 m", |app| {
        cars.iter()
            .any(|&car| matches!(lane_source(app, car), (edge, progress) if edge == north && progress > R1_NORTH_MM))
    });
    let past = *cars
        .iter()
        .find(|&&car| lane_source(&app, car).0 == north && lane_source(&app, car).1 > R1_NORTH_MM)
        .unwrap();
    let before_generation = generation(&app);
    request(&mut app);
    zero_frame(&mut app);
    let refused = cutover(&app);
    assert_eq!(refused.current, RevisionName::R2);
    assert_eq!(refused.state, CutoverState::AwaitingRetry);
    assert_eq!(
        refused.last_refusal.unwrap().reason,
        RefusalReason::VehicleRevalidation
    );
    assert_eq!(generation(&app), before_generation);
    assert!(lane_source(&app, past).1 > R1_NORTH_MM);
    until(&mut app, 200, "重试成功", |app| {
        cutover(app).current == RevisionName::R1
    });
    assert_eq!(cutover(&app).pending, None);
    assert_eq!(cutover(&app).state, CutoverState::Idle);
    assert_eq!(world_revision(&app), revision_id(&app, RevisionName::R1));
    assert_ne!(
        lane_source(&app, past).0,
        scene::edge(&root(&app, RevisionName::R1), "north").unwrap()
    );
    assert_eq!(ring(&app), cars);
}

#[test]
fn r10_repeated_round_trips_are_consistent() {
    let mut app = app();
    let cars = ring(&app);
    let bound = entities(&app);
    let start = generation(&app);
    for round in 0..4_u64 {
        frames(&mut app, 40);
        request(&mut app);
        until(&mut app, 300, "进入 R2", |app| {
            cutover(app).current == RevisionName::R2
        });
        assert_eq!(world_revision(&app), revision_id(&app, RevisionName::R2));
        frames(&mut app, 40);
        request(&mut app);
        until(&mut app, 300, "回到 R1", |app| {
            cutover(app).current == RevisionName::R1
        });
        assert_eq!(world_revision(&app), revision_id(&app, RevisionName::R1));
        assert_eq!(generation(&app), start + 2 * (round + 1));
        assert_eq!(cutover(&app).successes as u64, 2 * (round + 1));
        assert_eq!(cutover(&app).events, 2 * (round + 1));
        assert_eq!(ring(&app), cars);
        for (&vehicle, &entity) in cars.iter().zip(&bound) {
            assert_eq!(session(&app).vehicle_entity(vehicle), Some(entity));
        }
        assert_eq!(fleet(&app).vehicles[SPUR_SLOT], None);
        assert_eq!(session(&app).last_error(), None);
    }
}

#[test]
fn r10_planned_ticks_drive_the_same_path() {
    let mut app = app_with(vec![30, 200]);
    until(&mut app, 400, "两次计划切换", |app| {
        cutover(app).successes == 2
    });
    assert_eq!(cutover(&app).current, RevisionName::R1);
    let refusal = cutover(&app).last_refusal.unwrap();
    assert_eq!(refusal.reason, RefusalReason::UnmappableLaneEdge);
    assert!(refusal.tick >= 200, "{refusal:?}");
}

#[test]
fn r11_zero_step_frame_cuts_over_without_advancing_the_tick() {
    let mut app = app();
    frames(&mut app, 10);
    let before = tick(&app);
    enter_r2(&mut app);
    assert_eq!(tick(&app), before);
    let summary = app.world().resource::<Summary>().current.clone().unwrap();
    assert_eq!(summary.steps, 0);
    assert_eq!(summary.tick, before);
    assert_eq!(
        app.world()
            .resource::<Presentation>()
            .poses
            .batch()
            .network_revision(),
        Some(revision_id(&app, RevisionName::R2))
    );
}

#[test]
fn r12_backlog_survives_the_cutover_and_drains_afterwards() {
    let mut app = app();
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        512,
    )));
    app.update();
    healthy(&app);
    let report = session(&app).frame_report();
    assert_eq!(report.steps_run(), 8);
    let backlog = report.backlog();
    assert_eq!(backlog, Duration::from_millis(512 - 8 * 16));
    let before = tick(&app);
    // 零时长帧仍按 catch-up 上限排空 backlog；切换只要没有清除它，这一帧就恰好少 8 拍。
    enter_r2(&mut app);
    let drained = backlog - 8 * DELTA;
    assert_eq!(session(&app).frame_report().steps_run(), 8);
    assert_eq!(session(&app).frame_report().backlog(), drained);
    assert_eq!(tick(&app), before + 8);
    let summary = app.world().resource::<Summary>().current.clone().unwrap();
    assert_eq!(summary.backlog, drained);
    let mut later_frames = 0;
    while !session(&app).frame_report().backlog().is_zero() {
        frame(&mut app);
        later_frames += 1;
        assert!(later_frames < 100, "backlog 未排空");
    }
    // 保留的每一拍都执行了，后续帧各自再贡献一拍。
    assert_eq!(
        tick(&app),
        before + backlog.as_millis() as u64 / 16 + later_frames
    );
}

#[test]
fn r13_mismatched_target_spatial_stops_and_leaves_presentation_untouched() {
    let mut app = app();
    frames(&mut app, 5);
    let (before, road_version) = (transforms(&app), app.world().resource::<RoadData>().version);
    let visible = app.world().resource::<Presentation>().visible.clone();
    let wrong = SpatialSession::bind(root(&app, RevisionName::R1))
        .unwrap()
        .unwrap();
    request(&mut app);
    host::attempt(app.world_mut(), wrong);
    assert!(
        app.world()
            .resource::<Health>()
            .failure
            .as_ref()
            .unwrap()
            .detail
            .contains("TargetSpatialRevisionMismatch")
    );
    let tick_at_failure = tick(&app);
    app.insert_resource(TimeUpdateStrategy::ManualDuration(DELTA));
    app.update();
    assert_eq!(tick(&app), tick_at_failure);
    assert_eq!(transforms(&app), before);
    assert_eq!(app.world().resource::<Presentation>().visible, visible);
    assert_eq!(app.world().resource::<RoadData>().version, road_version);
    assert_eq!(world_revision(&app), revision_id(&app, RevisionName::R1));
    assert!(app.world().resource::<Summary>().current.is_none());
    assert!(exit_code(&app));
}

#[test]
fn r13_batch_and_road_revision_mismatch_stops_the_whole_batch() {
    let mut app = app();
    frames(&mut app, 5);
    let before = transforms(&app);
    let other = revision_id(&app, RevisionName::R2);
    app.world_mut().resource_mut::<RoadData>().revision = other;
    app.insert_resource(TimeUpdateStrategy::ManualDuration(DELTA));
    app.update();
    assert!(
        app.world()
            .resource::<Health>()
            .failure
            .as_ref()
            .unwrap()
            .detail
            .contains("批次修订与路面修订不一致")
    );
    assert_eq!(transforms(&app), before);
    assert!(exit_code(&app));
}

#[test]
fn r13_stale_context_never_reaches_the_entities() {
    let mut app = app();
    frames(&mut app, 5);
    let old = session(&app).consumption_context();
    enter_r2(&mut app);
    let before = transforms(&app);
    host::present_with(app.world_mut(), old);
    assert!(
        app.world()
            .resource::<Health>()
            .failure
            .as_ref()
            .unwrap()
            .detail
            .contains("过期消费上下文")
    );
    assert_eq!(transforms(&app), before);
}

#[test]
fn r13_unmappable_with_nothing_to_clear_is_a_failure() {
    let mut app = app();
    assert!(host::clear_spur(app.world_mut()).is_err());
}

#[test]
fn classification_rejects_combinations_outside_the_table() {
    let lane = Some(PoseSource::Lane {
        edge: LaneEdgeOrdinal::from_raw(0),
        progress_mm: 0,
    });
    let parking = Some(PoseSource::Parking {
        space: ParkingSpaceOrdinal::from_raw(0),
    });
    let bound = Some(ParkingBinding::Occupied(ParkingTarget::VirtualPool(
        ParkingFacilityOrdinal::from_raw(0),
    )));
    assert_eq!(classify(VehicleStatus::Active, None, lane, true), Ok(true));
    assert_eq!(
        classify(VehicleStatus::Completed, None, None, false),
        Ok(false)
    );
    assert!(classify(VehicleStatus::Active, None, lane, false).is_err());
    assert!(classify(VehicleStatus::Completed, None, None, true).is_err());
    assert!(classify(VehicleStatus::Active, None, None, false).is_err());
    assert!(classify(VehicleStatus::Parked, None, parking, true).is_err());
    assert!(classify(VehicleStatus::Parked, bound, None, false).is_err());
}

#[test]
fn hud_reports_revision_state_and_counters() {
    let mut app = app();
    let hud = |app: &App| {
        app.world()
            .resource::<Summary>()
            .current
            .as_ref()
            .unwrap()
            .hud()
    };
    let text = hud(&app);
    let prefix: String = revision_id(&app, RevisionName::R1).as_digest().as_bytes()[..4]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    for expected in [
        format!("revision R1  {prefix}  generation"),
        "cutover: idle".into(),
        "cutovers 0  events 0  stale batches rejected 0".into(),
        "cutover refused: none".into(),
        "ring (white) 4 visible".into(),
        "spur (orange) none".into(),
        "C: cutover".into(),
    ] {
        assert!(text.contains(&expected), "{expected}\n{text}");
    }
    frames(&mut app, 3);
    enter_r2(&mut app);
    let text = hud(&app);
    assert!(text.contains("revision R2"), "{text}");
    assert!(
        text.contains("cutovers 1  events 1  stale batches rejected 1"),
        "{text}"
    );
    assert!(text.contains("spur (orange) driving"), "{text}");
}
