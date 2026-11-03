use std::{num::NonZeroU32, sync::Arc, time::Duration};

use bevy_app::App;
use bevy_math::{Mat4, Vec3};
use bevy_time::{Time, TimePlugin, TimeUpdateStrategy, Virtual};
use bevy_transform::{TransformPlugin, components::Transform};
use laneflow_bevy::{
    LaneFlowCommittedPoseBatch, LaneFlowConsumptionContext, LaneFlowSession, LaneFlowSessionConfig,
};
use laneflow_runtime::{
    CommittedNetworkSource, ExecutionConfig, PublishedLfcaReference, TrafficWorld, VehicleHandle,
    VehicleState, VehicleStatus, WorldConfig,
};
use laneflow_spatial::{CanonicalPoseF32, FramePlacementToken};
use laneflow_static_contract::LaneEdgeOrdinal;

use super::{
    culling_cli::{self, CameraPreset, Cli},
    culling_host as host,
    culling_scene::{self as scene, SceneConfig},
    culling_view::{LaneChunks, ViewVolume},
};

const DELTA: Duration = Duration::from_millis(16);

fn small(cells: usize) -> SceneConfig {
    SceneConfig {
        cells,
        vehicles_per_cell: 16,
        route_legs: 4,
    }
}

fn app(config: SceneConfig, view: ViewVolume) -> App {
    let mut app = App::new();
    app.add_plugins((TimePlugin, TransformPlugin));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    app.world_mut()
        .resource_mut::<Time<Virtual>>()
        .set_max_delta(Duration::from_secs(60));
    host::install(&mut app, scene::build(config).unwrap(), view).unwrap();
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

fn frame(app: &mut App, delta: Duration) {
    app.insert_resource(TimeUpdateStrategy::ManualDuration(delta));
    app.update();
    healthy(app);
}

fn run(app: &mut App, frames: usize) {
    for _ in 0..frames {
        frame(app, DELTA);
    }
}

/// 俯视一个以 `center` 为中心的正方形范围；比透视更容易写出可判定的边界。
fn top_down(center: Vec3, half_width: f32) -> ViewVolume {
    let world_from_view = Transform::from_translation(center + Vec3::Y * 500.0)
        .looking_to(Vec3::NEG_Y, Vec3::NEG_Z)
        .to_matrix();
    // 观察方向是 -Z：把 z 从 -1000..=0 映射到裁剪空间的 1..=0。
    let clip_from_view = Mat4::from_scale(Vec3::new(
        1.0 / half_width,
        1.0 / half_width,
        -1.0 / 1_000.0,
    ));
    ViewVolume::from_clip_from_world(clip_from_view * world_from_view.inverse())
}

fn nowhere() -> ViewVolume {
    top_down(Vec3::new(1.0e6, 0.0, 1.0e6), 1.0)
}

fn full_poses(app: &mut App) -> LaneFlowCommittedPoseBatch {
    let token = app.world().resource::<host::Fleet>().token;
    let mut poses = LaneFlowCommittedPoseBatch::new();
    app.world_mut()
        .resource_mut::<LaneFlowSession>()
        .extract_committed_pose_batch(token, &mut poses)
        .unwrap();
    poses
}

fn states(app: &App) -> Vec<Option<VehicleState>> {
    let world = app.world();
    let session = world.resource::<LaneFlowSession>();
    world
        .resource::<host::Fleet>()
        .vehicles
        .iter()
        .map(|vehicle| session.world().vehicle(*vehicle))
        .collect()
}

fn visible_count(app: &App) -> usize {
    let culling = app.world().resource::<host::Culling>();
    culling.visible.iter().filter(|visible| **visible).count()
}

fn slot(app: &App, vehicle: VehicleHandle) -> usize {
    app.world()
        .resource::<host::Fleet>()
        .vehicles
        .iter()
        .position(|candidate| *candidate == vehicle)
        .unwrap()
}

fn position(pose: CanonicalPoseF32) -> Vec3 {
    let point = pose.position();
    Vec3::new(point.x(), point.y(), point.z())
}

fn summary(app: &App) -> host::FrameSummary {
    app.world()
        .resource::<host::Summary>()
        .current
        .clone()
        .unwrap()
}

#[test]
fn e01_cli_accepts_bounds_and_rejects_invalid_arguments() {
    let parse = |args: &[&str]| culling_cli::parse(args.iter().map(ToString::to_string));
    let Cli::Run(default) = parse(&[]).unwrap() else {
        panic!("default must run");
    };
    assert_eq!(default.cells, 9);
    assert_eq!(default.camera, CameraPreset::Cell);
    assert_eq!(default.capture, None);
    for cells in ["1", "32"] {
        assert!(parse(&["--cells", cells]).is_ok());
    }
    assert_eq!(parse(&["--help"]).unwrap(), Cli::Help);
    assert!(!culling_cli::HELP.is_empty());
    let Cli::Run(capture) = parse(&["--screenshot", "a.png", "--camera", "frozen"]).unwrap() else {
        panic!("capture must run");
    };
    assert_eq!(capture.camera, CameraPreset::Frozen);
    let request = capture.capture.unwrap();
    assert_eq!(request.path.to_str(), Some("a.png"));
    assert_eq!(request.tick, 1_500);
    for bad in [
        &["--cells", "0"][..],
        &["--cells", "33"],
        &["--cells", "x"],
        &["--cells"],
        &["--cells", "1", "--cells", "2"],
        &["--camera", "top"],
        &["--screenshot", "a.jpg"],
        &["--at-tick", "10"],
        &["--screenshot", "a.png", "--at-tick", "50001"],
        &["--unknown"],
    ] {
        assert!(parse(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn e01_startup_binds_every_vehicle_and_repeats_the_same_plan() {
    let first = app(small(2), ViewVolume::ALL);
    let second = app(small(2), ViewVolume::ALL);
    assert_eq!(states(&first), states(&second));
    let fleet = first.world().resource::<host::Fleet>();
    assert_eq!(fleet.vehicles.len(), 32);
    let session = first.world().resource::<LaneFlowSession>();
    for (vehicle, entity) in fleet.vehicles.iter().zip(&fleet.entities) {
        assert_eq!(session.vehicle_entity(*vehicle), Some(*entity));
    }
    assert_eq!(visible_count(&first), 32);
}

#[test]
fn e01_default_density_fits_the_largest_grid() {
    let scene = scene::build(SceneConfig::new(32)).unwrap();
    assert_eq!(scene.vehicles.len(), 32 * 312);
    assert!(scene.min.x < scene.max.x && scene.min.z < scene.max.z);
}

#[test]
fn e02_selection_is_conservative_and_a_local_view_selects_fewer() {
    let mut app = app(small(4), ViewVolume::ALL);
    run(&mut app, 30);
    let total = app.world().resource::<host::Fleet>().vehicles.len();
    let anchor = position(full_poses(&mut app).batch().records()[0].pose());
    for half_width in [30.0, 120.0, 260.0] {
        let view = top_down(anchor, half_width);
        app.insert_resource(view.clone());
        run(&mut app, 3);
        let poses = full_poses(&mut app);
        let length = app.world().resource::<host::Fleet>().vehicle_length;
        let selected = app.world().resource::<host::Culling>().selected.clone();
        for (vehicle, record) in poses.vehicles().iter().zip(poses.batch().records()) {
            let point = position(record.pose());
            // 车身从前保险杠向后延伸，车长范围内都可能在视域中。
            if view.intersects(point - length, point + length) {
                assert!(selected.contains(vehicle), "漏选 {vehicle:?} at {point:?}");
            }
        }
        assert!(
            selected.len() < total,
            "half_width={half_width}: {} of {total}",
            selected.len()
        );
        assert!(!selected.is_empty());
    }
}

#[test]
fn e03_periodic_audit_matches_full_extraction_and_output_keeps_selection_order() {
    let mut app = app(small(2), ViewVolume::ALL);
    run(&mut app, 2);
    let anchor = position(full_poses(&mut app).batch().records()[0].pose());
    app.insert_resource(top_down(anchor, 120.0));
    run(&mut app, 200);
    let culling = app.world().resource::<host::Culling>();
    assert!(culling.audits >= 3, "audits={}", culling.audits);
    assert_eq!(culling.poses.vehicles(), culling.selected);
    let selected = culling.poses.vehicles().to_vec();
    let output: Vec<_> = culling
        .poses
        .batch()
        .records()
        .iter()
        .map(|record| record.pose())
        .collect();
    let full = full_poses(&mut app);
    for (vehicle, pose) in selected.iter().zip(output) {
        let row = full
            .vehicles()
            .iter()
            .position(|candidate| candidate == vehicle)
            .unwrap();
        assert_eq!(full.batch().records()[row].pose(), pose);
    }
    let summary = summary(&app);
    assert!(summary.full_time.is_some());
    assert!(summary.frame >= 180);
    assert!(summary.steps <= 8);
    assert!(summary.backlog < DELTA);
    assert!(summary.hud().contains("not performance evidence"));
}

#[test]
fn hud_reports_whether_the_selection_view_is_frozen_in_the_same_frame() {
    let mut app = app(small(1), ViewVolume::ALL);
    assert!(summary(&app).hud().contains("follows camera"));
    app.world_mut().resource_mut::<host::ViewControl>().frozen = true;
    frame(&mut app, Duration::ZERO);
    assert!(summary(&app).hud().contains("FROZEN"));
    app.world_mut().resource_mut::<host::ViewControl>().frozen = false;
    frame(&mut app, Duration::ZERO);
    assert!(summary(&app).hud().contains("follows camera"));
}

#[test]
fn e04_leaving_and_returning_keeps_identity_and_shows_the_latest_pose() {
    let mut app = app(small(2), ViewVolume::ALL);
    run(&mut app, 5);
    let binding: Vec<_> = {
        let world = app.world();
        let session = world.resource::<LaneFlowSession>();
        world
            .resource::<host::Fleet>()
            .vehicles
            .iter()
            .map(|vehicle| session.vehicle_entity(*vehicle))
            .collect()
    };
    app.insert_resource(nowhere());
    frame(&mut app, DELTA);
    assert_eq!(visible_count(&app), 0);
    let hidden = committed_view(&app).1;
    run(&mut app, 300);
    // 隐藏期间不写 Transform，也不改变绑定。
    assert_eq!(committed_view(&app).1, hidden);
    {
        let world = app.world();
        let session = world.resource::<LaneFlowSession>();
        let fleet = world.resource::<host::Fleet>();
        for (index, vehicle) in fleet.vehicles.iter().enumerate() {
            assert_eq!(session.vehicle_entity(*vehicle), binding[index]);
        }
    }
    app.insert_resource(ViewVolume::ALL);
    frame(&mut app, Duration::ZERO);
    assert_eq!(visible_count(&app), 32);
    let poses = full_poses(&mut app);
    let mut moved = 0;
    for (vehicle, record) in poses.vehicles().iter().zip(poses.batch().records()) {
        let index = slot(&app, *vehicle);
        let entity = app.world().resource::<host::Fleet>().entities[index];
        let transform = app.world().get::<Transform>(entity).unwrap();
        assert_eq!(
            *transform,
            host::transform_from_pose(record.pose()).unwrap()
        );
        moved += usize::from(*transform != hidden[index]);
    }
    assert!(moved > 0, "车辆在隐藏期间应继续推进");
}

#[test]
fn e05_empty_selection_succeeds_hides_everything_and_traffic_continues() {
    let mut app = app(small(1), nowhere());
    run(&mut app, 20);
    let summary = summary(&app);
    assert_eq!((summary.selected, summary.output), (0, 0));
    assert_eq!(summary.total, 16);
    assert!(summary.tick > 0);
    assert_eq!(visible_count(&app), 0);
    let culling = app.world().resource::<host::Culling>();
    assert!(culling.poses.batch().records().is_empty());
    app.insert_resource(ViewVolume::ALL);
    frame(&mut app, Duration::ZERO);
    assert_eq!(visible_count(&app), 16);
}

#[test]
fn e06_completed_vehicles_hide_keep_binding_and_leave_the_selection() {
    let mut app = app(
        SceneConfig {
            cells: 2,
            vehicles_per_cell: 8,
            route_legs: 1,
        },
        ViewVolume::ALL,
    );
    let mut completed = None;
    for _ in 0..30_000 {
        frame(&mut app, DELTA * 4);
        let world = app.world();
        let session = world.resource::<LaneFlowSession>();
        completed = world
            .resource::<host::Fleet>()
            .vehicles
            .iter()
            .copied()
            .find(|vehicle| {
                session
                    .world()
                    .vehicle(*vehicle)
                    .is_some_and(|state| state.status() == VehicleStatus::Completed)
            });
        if completed.is_some() {
            break;
        }
    }
    let completed = completed.expect("有界窗口内应有车辆完成路线");
    let index = slot(&app, completed);
    frame(&mut app, Duration::ZERO);
    let world = app.world();
    let culling = world.resource::<host::Culling>();
    assert!(!culling.visible[index]);
    assert!(!culling.selected.contains(&completed));
    assert_eq!(
        world
            .resource::<LaneFlowSession>()
            .vehicle_entity(completed),
        Some(world.resource::<host::Fleet>().entities[index])
    );
    assert!(culling.visible.iter().any(|visible| *visible));
}

fn committed_view(app: &App) -> (Vec<bool>, Vec<Transform>) {
    let world = app.world();
    let fleet = world.resource::<host::Fleet>();
    (
        world.resource::<host::Culling>().visible.clone(),
        fleet
            .entities
            .iter()
            .map(|entity| *world.get::<Transform>(*entity).unwrap())
            .collect(),
    )
}

/// 车辆已隐藏且世界继续推进的基线：此后一旦恢复全量视域，任何部分写入都能被看出来。
fn rejection_rig() -> (App, (Vec<bool>, Vec<Transform>)) {
    let mut app = app(small(1), ViewVolume::ALL);
    run(&mut app, 5);
    app.insert_resource(nowhere());
    run(&mut app, 120);
    let before = committed_view(&app);
    assert!(before.0.iter().all(|visible| !visible));
    app.insert_resource(ViewVolume::ALL);
    (app, before)
}

/// 触发失败后断言表现零写入、记录首错，并请求非零退出。
fn assert_rejected(mut app: App, before: &(Vec<bool>, Vec<Transform>), expected: &str) {
    assert_eq!(&committed_view(&app), before);
    let detail = app
        .world()
        .resource::<host::Health>()
        .failure
        .as_ref()
        .expect("必须记录失败")
        .detail
        .clone();
    assert!(detail.contains(expected), "{detail}");
    assert!(
        app.world()
            .resource::<host::Health>()
            .failure
            .as_ref()
            .is_some_and(|failure| failure.tick > 0)
    );
    assert!(app.world().resource::<host::Summary>().current.is_none());
    host::exit_on_failure(app.world_mut());
    assert!(app.should_exit().is_some_and(|exit| exit.is_error()));
}

/// 另一世界的消费上下文：同修订、同策略，只有 `world_id` 不同。
fn foreign_context(app: &App) -> LaneFlowConsumptionContext {
    let session = app.world().resource::<LaneFlowSession>();
    let revision = session.world().revision();
    let origin = revision.canonical_origin();
    let world = TrafficWorld::install(
        Arc::clone(&revision),
        WorldConfig::new(1, 1, 1_024, 1_024, 16),
        ExecutionConfig::new(NonZeroU32::MIN),
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "scenario://culling-foreign",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .unwrap(),
        },
        930,
        session.world().policy_selection(),
    )
    .unwrap();
    LaneFlowSession::new(world, None, LaneFlowSessionConfig::new(NonZeroU32::MIN))
        .unwrap()
        .consumption_context()
}

#[test]
fn e07_stale_context_rejects_the_whole_batch() {
    let (mut app, before) = rejection_rig();
    let foreign = foreign_context(&app);
    host::present_with(app.world_mut(), foreign);
    assert_rejected(app, &before, "StalePoseSelectionContext");
}

#[test]
fn e07_stale_placement_token_rejects_the_whole_batch() {
    let (mut app, before) = rejection_rig();
    let root = app.world().resource::<host::Fleet>().root;
    app.world_mut()
        .entity_mut(root)
        .insert(host::Placement(FramePlacementToken::new(99)));
    let context = app
        .world()
        .resource::<LaneFlowSession>()
        .consumption_context();
    host::present_with(app.world_mut(), context);
    assert_rejected(app, &before, "placement token");
}

#[test]
fn e07_lost_binding_rejects_the_whole_batch_even_for_later_vehicles() {
    let (mut app, before) = rejection_rig();
    let vehicle = *app
        .world()
        .resource::<host::Fleet>()
        .vehicles
        .last()
        .unwrap();
    app.world_mut()
        .resource_mut::<LaneFlowSession>()
        .unbind_vehicle(vehicle)
        .unwrap();
    let context = app
        .world()
        .resource::<LaneFlowSession>()
        .consumption_context();
    host::present_with(app.world_mut(), context);
    assert_rejected(app, &before, "位姿缺少绑定");
}

#[test]
fn e07_non_finite_axes_are_rejected_before_any_write() {
    for (position, tangent, up) in [
        (Vec3::NAN, Vec3::X, Vec3::Y),
        (Vec3::ZERO, Vec3::INFINITY, Vec3::Y),
        (Vec3::ZERO, Vec3::ZERO, Vec3::Y),
        (Vec3::ZERO, Vec3::Y, Vec3::Y),
    ] {
        assert!(host::transform_from_axes(position, tangent, up).is_err());
    }
    assert!(host::transform_from_axes(Vec3::ZERO, Vec3::X, Vec3::Y).is_ok());
}

#[test]
fn e08_selection_does_not_change_traffic() {
    let mut full = app(small(2), ViewVolume::ALL);
    let mut hidden = app(small(2), nowhere());
    let mut moving = app(small(2), ViewVolume::ALL);
    let anchor = position(full_poses(&mut moving).batch().records()[0].pose());
    let signals = |app: &App| {
        app.world()
            .resource::<LaneFlowSession>()
            .world()
            .committed_signal_groups()
            .as_slice()
            .to_vec()
    };
    for index in 0..600_usize {
        moving.insert_resource(if index % 90 < 45 {
            top_down(anchor, 40.0 + index as f32 * 0.2)
        } else {
            nowhere()
        });
        let delta = DELTA * [1, 0, 2, 4, 1][index % 5];
        frame(&mut full, delta);
        frame(&mut hidden, delta);
        frame(&mut moving, delta);
        let expected = states(&full);
        assert_eq!(states(&hidden), expected, "frame {index}");
        assert_eq!(states(&moving), expected, "frame {index}");
        assert_eq!(signals(&hidden), signals(&full));
        assert_eq!(signals(&moving), signals(&full));
    }
    assert!(
        full.world()
            .resource::<LaneFlowSession>()
            .world()
            .tick_index()
            > 0
    );
}

#[test]
fn chunks_report_lanes_outside_the_table_and_clamp_progress_to_the_last_chunk() {
    let lanes = vec![vec![Vec3::ZERO, Vec3::new(60.0, 0.0, 0.0)]];
    let mut chunks = LaneChunks::build(&lanes, &[60_000], 1.0).unwrap();
    let edge = LaneEdgeOrdinal::from_raw(0);
    chunks.refresh(&ViewVolume::ALL);
    assert_eq!(chunks.is_visible(edge, 60_000), Some(true));
    assert_eq!(chunks.is_visible(LaneEdgeOrdinal::from_raw(1), 0), None);
    chunks.refresh(&top_down(Vec3::ZERO, 10.0));
    assert_eq!(chunks.is_visible(edge, 0), Some(true));
    assert_eq!(chunks.is_visible(edge, 59_000), Some(false));
    assert!(LaneChunks::build(&lanes, &[], 1.0).is_err());
}
