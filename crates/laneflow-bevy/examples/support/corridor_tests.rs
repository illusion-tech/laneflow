use std::{collections::HashSet, time::Duration};

use bevy_app::{App, AppExit};
use bevy_ecs::{
    hierarchy::ChildOf,
    message::Messages,
    resource::Resource,
    schedule::IntoScheduleConfigs,
    world::{Mut, World},
};
use bevy_math::{Vec2, Vec3};
use bevy_time::{Time, TimePlugin, TimeUpdateStrategy, Virtual};
use bevy_transform::{TransformPlugin, components::Transform};
use laneflow_bevy::{
    LaneFlowAdapterError, LaneFlowFixed, LaneFlowFixedSet, LaneFlowOuterFrame, LaneFlowSession,
    LaneFlowVehicleReplaceOutcome, replace_completed_vehicle,
};
use laneflow_runtime::{VehicleHandle, VehicleSpawnInput, VehicleStatus};
use laneflow_scenario::signalized_corridor::{
    CatalogPolicySelection, CorridorCatalog, CorridorPopulationCapacities, CorridorPopulationConfig,
};
use sha2::{Digest, Sha256};

use super::{
    corridor_camera::Orbit, corridor_cli, corridor_host as host, corridor_replay as replay,
    corridor_scene as scene,
};

fn app(target: usize, seed: u64) -> App {
    let mut app = App::new();
    app.add_plugins((TimePlugin, TransformPlugin));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    app.world_mut()
        .resource_mut::<Time<Virtual>>()
        .set_max_delta(Duration::from_secs(60));
    host::install(
        &mut app,
        scene::build(CorridorPopulationConfig::try_new(target, seed).unwrap()).unwrap(),
    )
    .unwrap();
    app.update();
    app
}

fn frame(app: &mut App, delta: Duration) {
    app.insert_resource(TimeUpdateStrategy::ManualDuration(delta));
    app.update();
    assert!(
        app.world().resource::<host::Health>().failure.is_none(),
        "{:?}",
        app.world().resource::<host::Health>().failure
    );
    assert_eq!(
        app.world()
            .resource::<host::Summary>()
            .current
            .as_ref()
            .unwrap()
            .delta,
        delta
    );
}

fn states(world: &World) -> Vec<laneflow_runtime::VehicleState> {
    let session = world.resource::<LaneFlowSession>();
    session
        .world()
        .live_vehicles()
        .iter()
        .map(|vehicle| session.world().vehicle(*vehicle).unwrap())
        .collect()
}

fn transforms(world: &World) -> Vec<Option<Transform>> {
    world
        .resource::<host::Proxies>()
        .entities
        .iter()
        .map(|entity| world.get::<Transform>(*entity).copied())
        .collect()
}

#[test]
fn c01_nine_startups_bind_all_vehicles_deterministically_without_overlap() {
    for target in [50, 100, 200] {
        for seed in [0, 73, u64::MAX] {
            let first = app(target, seed);
            let second = app(target, seed);
            let world = first.world();
            let session = world.resource::<LaneFlowSession>();
            let controller = &world.resource::<host::Population>().0;
            assert_eq!(session.world().tick_index(), 0);
            assert_eq!(controller.counts().running, target);
            assert_eq!(controller.counts().pending, 0);
            assert_eq!(
                controller.rng_state(),
                second.world().resource::<host::Population>().0.rng_state()
            );
            assert_eq!(states(world), states(second.world()));
            let proxies = world.resource::<host::Proxies>();
            assert_eq!(proxies.entities.len(), target);
            assert_eq!(
                proxies.entities.iter().collect::<HashSet<_>>().len(),
                target
            );
            for (slot, &entity) in proxies.entities.iter().enumerate() {
                let vehicle = controller.logical_vehicle(slot).unwrap();
                assert_eq!(session.vehicle_entity(vehicle), Some(entity));
                assert!(session.world().vehicle(vehicle).is_some());
                let model = world.get::<Transform>(proxies.models[slot]).unwrap();
                assert_eq!(model.translation.z, proxies.vehicle_length * 0.5);
            }
            let states = states(world);
            for (index, a) in states.iter().enumerate() {
                let edge_a =
                    session.world().route_edges(a.route()).unwrap()[a.route_edge_index() as usize];
                for b in &states[index + 1..] {
                    let edge_b = session.world().route_edges(b.route()).unwrap()
                        [b.route_edge_index() as usize];
                    if edge_a == edge_b {
                        let (behind, ahead) = if a.progress_mm() < b.progress_mm() {
                            (a, b)
                        } else {
                            (b, a)
                        };
                        assert!(
                            ahead.progress_mm() - behind.progress_mm() >= ahead.length_mm(),
                            "initial bodies overlap"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn c02_cli_and_malformed_catalog_fail_before_an_app_exists() {
    let parse = |args: &[&str]| corridor_cli::parse(args.iter().map(|arg| (*arg).to_owned()));
    assert_eq!(
        parse(&[]).unwrap(),
        corridor_cli::Cli::Run(CorridorPopulationConfig::default())
    );
    assert_eq!(parse(&["--help"]).unwrap(), corridor_cli::Cli::Help);
    assert!(corridor_cli::HELP.contains("50..=200"));
    assert!(parse(&["--vehicles", "200", "--seed", "18446744073709551615"]).is_ok());
    for args in [
        vec!["--vehicles", "49"],
        vec!["--vehicles", "201"],
        vec!["--vehicles"],
        vec!["--seed"],
        vec!["--unknown"],
        vec!["--seed", "1", "--seed", "2"],
        vec!["--vehicles", "100", "--vehicles", "100"],
        vec!["--seed", "18446744073709551616"],
        vec!["--seed", "-1"],
        vec!["--seed", "+1"],
        vec!["--vehicles", "1.0"],
        vec!["--help", "--help"],
    ] {
        assert!(parse(&args).is_err(), "invalid CLI: {args:?}");
    }
    let catalog: CorridorCatalog = toml::from_str(scene::CATALOG).unwrap();
    let mut bad = catalog.clone();
    bad.catalog_version = "0.3".into();
    assert!(scene::build_catalog(CorridorPopulationConfig::default(), &bad).is_err());
    bad = catalog.clone();
    bad.policy_selection = CatalogPolicySelection::NotRequired {};
    assert!(scene::build_catalog(CorridorPopulationConfig::default(), &bad).is_err());
    bad = catalog;
    bad.spawn_slots.truncate(49);
    assert!(scene::build_catalog(CorridorPopulationConfig::default(), &bad).is_err());
}

#[test]
fn c03_real_app_keeps_backlog_and_fractional_time() {
    let mut app = app(100, 0);
    assert_eq!(
        app.world()
            .resource::<host::BoundaryTrace>()
            .lifecycle_calls,
        0
    );
    frame(&mut app, Duration::from_micros(499));
    let summary = app
        .world()
        .resource::<host::Summary>()
        .current
        .as_ref()
        .unwrap();
    assert_eq!(summary.tick, 0);
    assert_eq!(summary.backlog, Duration::from_micros(499));
    frame(&mut app, Duration::from_micros(15_501));
    assert_eq!(
        app.world()
            .resource::<host::Summary>()
            .current
            .as_ref()
            .unwrap()
            .tick,
        1
    );
    frame(&mut app, Duration::from_millis(160));
    let summary = app
        .world()
        .resource::<host::Summary>()
        .current
        .as_ref()
        .unwrap();
    assert_eq!(summary.steps, 8);
    assert_eq!(summary.backlog, Duration::from_millis(32));
    assert!(summary.limited);
    frame(&mut app, Duration::ZERO);
    let summary = app
        .world()
        .resource::<host::Summary>()
        .current
        .as_ref()
        .unwrap();
    assert_eq!(summary.steps, 2);
    assert_eq!(summary.backlog, Duration::ZERO);
    assert!(!summary.limited);
    for steps in [1, 4, 8] {
        let before = app
            .world()
            .resource::<host::BoundaryTrace>()
            .lifecycle_calls;
        frame(&mut app, Duration::from_millis(16 * steps));
        let trace = app.world().resource::<host::BoundaryTrace>();
        assert_eq!(trace.lifecycle_calls - before, steps);
        assert_eq!(trace.lifecycle_calls, trace.observe_calls);
    }
}

#[test]
fn c10_every_pose_matches_canonical_front_bumper_forward_and_up() {
    let mut app = app(200, 0);
    let mut directions = [false; 3];
    for _ in 0..750 {
        frame(&mut app, Duration::from_millis(128));
        let world = app.world();
        let poses = &world.resource::<host::Presentation>().poses;
        assert!(!poses.vehicles().is_empty());
        let session = world.resource::<LaneFlowSession>();
        for (&vehicle, record) in poses.vehicles().iter().zip(poses.batch().records()) {
            let transform = world
                .get::<Transform>(session.vehicle_entity(vehicle).unwrap())
                .unwrap();
            let pose = record.pose();
            let p = pose.position();
            let t = pose.tangent();
            let u = pose.up();
            assert!(
                transform
                    .translation
                    .distance(Vec3::new(p.x(), p.y(), p.z()))
                    < 0.000_01
            );
            assert!(transform.forward().dot(Vec3::new(t.x(), t.y(), t.z())) > 0.999_99);
            assert!(transform.up().dot(Vec3::new(u.x(), u.y(), u.z())) > 0.999_99);
            let state = session.world().vehicle(vehicle).unwrap();
            let revision = session.world().revision();
            let geometry = revision.spatial().unwrap().lane_pose().unwrap();
            let edges = session.world().route_edges(state.route()).unwrap();
            let initial = geometry.lane_geometry(edges[0]).unwrap().points();
            let end = geometry
                .lane_geometry(*edges.last().unwrap())
                .unwrap()
                .points();
            let direction = |points: &[laneflow_static_network::CanonicalPoint]| {
                Vec3::new(
                    points[1].x - points[0].x,
                    points[1].y - points[0].y,
                    points[1].z - points[0].z,
                )
                .normalize()
            };
            let start = direction(initial);
            let turn = start.cross(direction(end)).y;
            if turn.abs() < 0.1 {
                directions[0] = true;
            } else if start.cross(Vec3::new(t.x(), t.y(), t.z())).y.abs() > 0.1 {
                directions[if turn > 0.0 { 1 } else { 2 }] = true;
            }
        }
        if directions.into_iter().all(|seen| seen) {
            break;
        }
    }
    assert!(
        directions.into_iter().all(|seen| seen),
        "must inspect straight, left and right turns"
    );
}

#[test]
fn c10_bad_entity_token_parent_and_nonfinite_transform_commit_no_partial_batch() {
    for fault in 0..5 {
        let mut app = app(100, 0);
        let world = app.world_mut();
        let root = world.resource::<host::Proxies>().root;
        let last = *world.resource::<host::Proxies>().entities.last().unwrap();
        match fault {
            0 => {
                world.despawn(last);
            }
            1 => {
                world.get_mut::<host::Placement>(root).unwrap().0 =
                    laneflow_spatial::FramePlacementToken::new(9);
            }
            2 => {
                world.entity_mut(last).remove::<ChildOf>();
            }
            3 => {
                world.get_mut::<Transform>(last).unwrap().translation.x = f32::INFINITY;
            }
            _ => {
                world.get_mut::<Transform>(root).unwrap().scale = Vec3::splat(2.0);
            }
        }
        let before = transforms(app.world());
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            16,
        )));
        app.update();
        assert_eq!(
            transforms(app.world()),
            before,
            "partial presentation commit: {fault}"
        );
        let health = app.world().resource::<host::Health>();
        assert_eq!(
            health.failure.as_ref().unwrap().stage,
            host::Stage::Presentation
        );
        assert!(health.exit_sent);
        assert!(app.world().resource::<host::Summary>().current.is_none());
        let tick = app
            .world()
            .resource::<LaneFlowSession>()
            .world()
            .tick_index();
        app.update();
        assert_eq!(
            app.world()
                .resource::<LaneFlowSession>()
                .world()
                .tick_index(),
            tick
        );
    }
    assert!(host::transform_from_axes(Vec3::ZERO, Vec3::splat(f32::NAN), Vec3::Y).is_err());
    assert!(host::transform_from_axes(Vec3::ZERO, Vec3::Y, Vec3::Y).is_err());
}

#[test]
fn c13_camera_uses_input_displacement_and_keeps_finite_limits() {
    let app = app(50, 0);
    let geometry = app.world().resource::<scene::Geometry>();
    let mut one = Orbit::new(geometry);
    let mut split = one.clone();
    one.apply(Vec2::new(24.0, -18.0), Vec2::ZERO, 0.0);
    for _ in 0..4 {
        split.apply(Vec2::new(6.0, -4.5), Vec2::ZERO, 0.0);
    }
    assert!(one.focus.distance(split.focus) < 0.000_01);
    one.apply(Vec2::ZERO, Vec2::new(80.0, 40.0), 0.0);
    for _ in 0..4 {
        split.apply(Vec2::ZERO, Vec2::new(20.0, 10.0), 0.0);
    }
    assert!((one.yaw - split.yaw).abs() < 0.000_01);
    one.apply(Vec2::ZERO, Vec2::ZERO, 4.0);
    for _ in 0..4 {
        split.apply(Vec2::ZERO, Vec2::ZERO, 1.0);
    }
    assert!((one.distance - split.distance).abs() < 0.001);
    one.apply(Vec2::splat(1.0e20), Vec2::splat(1.0e20), -1.0e20);
    assert!(one.transform().translation.is_finite());
    assert!(one.transform().rotation.is_finite());
    assert_eq!(geometry.lane_width, 3.5);
    assert!(!geometry.lanes.is_empty());
    for lamp in &geometry.lamps {
        assert!(lamp.position.is_finite() && lamp.tangent.is_finite());
    }
}

#[derive(Resource)]
struct Audit {
    handles: Vec<VehicleHandle>,
    pending: Vec<VehicleHandle>,
    capacities: CorridorPopulationCapacities,
    initial_entities: Vec<bevy_ecs::entity::Entity>,
    initial_models: Vec<bevy_ecs::entity::Entity>,
    entity_count: u32,
    tick_hashes: Option<Vec<[u8; 32]>>,
    replacements: usize,
    completed: usize,
}

fn add_audit(app: &mut App, record_trace: bool) {
    let world = app.world();
    let population = &world.resource::<host::Population>().0;
    let proxies = world.resource::<host::Proxies>();
    let audit = Audit {
        handles: (0..population.counts().target)
            .map(|slot| population.logical_vehicle(slot).unwrap())
            .collect(),
        pending: Vec::new(),
        capacities: population.capacities(),
        initial_entities: proxies.entities.clone(),
        initial_models: proxies.models.clone(),
        entity_count: world.entities().len(),
        tick_hashes: record_trace.then(Vec::new),
        replacements: 0,
        completed: 0,
    };
    app.insert_resource(audit).add_systems(
        LaneFlowFixed,
        audit_tick
            .after(host::observe)
            .in_set(LaneFlowFixedSet::Observe),
    );
}

fn audit_tick(world: &mut World) {
    world.resource_scope(|world, mut audit: Mut<Audit>| {
        let session = world.resource::<LaneFlowSession>();
        let runtime = session.world();
        let population = &world.resource::<host::Population>().0;
        let proxies = world.resource::<host::Proxies>();
        let counts = population.counts();
        assert_eq!(counts.running + counts.pending, counts.target);
        assert!(counts.pending <= counts.target);
        assert_eq!(population.capacities(), audit.capacities);
        assert_eq!(runtime.live_vehicles().len(), counts.target);
        assert_eq!(population.last_consumed_tick(), runtime.tick_index());
        assert_eq!(proxies.entities, audit.initial_entities);
        assert_eq!(proxies.models, audit.initial_models);
        assert_eq!(world.entities().len(), audit.entity_count);
        let pending = population.pending_vehicles();
        let logical: Vec<_> = (0..counts.target)
            .map(|slot| population.logical_vehicle(slot).unwrap())
            .collect();
        for (slot, &new) in logical.iter().enumerate() {
            assert_eq!(
                session.vehicle_entity(new),
                Some(audit.initial_entities[slot])
            );
            assert_eq!(
                world
                    .get::<ChildOf>(audit.initial_models[slot])
                    .unwrap()
                    .parent(),
                audit.initial_entities[slot]
            );
            let old = audit.handles[slot];
            if old != new {
                assert!(
                    audit.pending.contains(&old),
                    "completion must be consumed before replacement"
                );
                assert!(runtime.vehicle(old).is_none());
                assert!(session.vehicle_entity(old).is_none());
                assert!(runtime.vehicle(new).is_some());
            }
        }
        audit.completed += pending
            .iter()
            .filter(|vehicle| !audit.pending.contains(vehicle))
            .count();
        let trace = world.resource::<host::BoundaryTrace>();
        assert_eq!(trace.lifecycle_calls, runtime.tick_index());
        assert_eq!(trace.observe_calls, runtime.tick_index());
        audit.replacements += trace.replacements.len();
        if let Some(hashes) = &mut audit.tick_hashes {
            let plans: Vec<_> = pending
                .iter()
                .map(|vehicle| population.pending_spawn_input(runtime, *vehicle).unwrap())
                .collect();
            let snapshot = format!(
                "{:?}",
                (
                    runtime.tick_index(),
                    runtime.time_ms(),
                    counts,
                    population.rng_state(),
                    population.last_consumed_tick(),
                    &logical,
                    states(world),
                    &pending,
                    plans,
                    runtime.committed_signal_groups(),
                    &trace.replacements
                )
            );
            hashes.push(Sha256::digest(snapshot.as_bytes()).into());
        }
        audit.handles = logical;
        audit.pending = pending;
    });
}

#[test]
fn c04_completion_waits_for_next_lifecycle_and_preserves_every_entity() {
    let mut app = app(50, 0);
    add_audit(&mut app, false);
    let mut saw_pending = false;
    for _ in 0..12_000 {
        frame(&mut app, Duration::from_millis(16));
        let pending = app
            .world()
            .resource::<host::Population>()
            .0
            .pending_vehicles();
        if let Some(&old) = pending.first() {
            saw_pending = true;
            let session = app.world().resource::<LaneFlowSession>();
            assert_eq!(
                session.world().vehicle(old).unwrap().status(),
                VehicleStatus::Completed
            );
            let entity = session.vehicle_entity(old).unwrap();
            let before = *app.world().get::<Transform>(entity).unwrap();
            frame(&mut app, Duration::ZERO);
            assert_eq!(*app.world().get::<Transform>(entity).unwrap(), before);
            assert_eq!(
                app.world()
                    .resource::<LaneFlowSession>()
                    .vehicle_entity(old),
                Some(entity)
            );
        }
        if saw_pending && app.world().resource::<Audit>().replacements >= 3 {
            break;
        }
    }
    let audit = app.world().resource::<Audit>();
    assert!(
        saw_pending && audit.replacements >= 3,
        "bounded run must replace actual completions"
    );
    println!(
        "C04 tick={} completions={} replacements={}",
        app.world()
            .resource::<LaneFlowSession>()
            .world()
            .tick_index(),
        audit.completed,
        audit.replacements
    );
}

fn run_replay(deltas: &[Duration]) -> (Vec<[u8; 32]>, host::FrameSummary, usize) {
    let mut app = app(100, 0);
    add_audit(&mut app, true);
    let total = Duration::from_millis(12_000 * 16);
    let mut submitted = Duration::ZERO;
    let mut frame_index = 0;
    while submitted < total {
        let delta = deltas[frame_index % deltas.len()].min(total - submitted);
        submitted += delta;
        frame(&mut app, delta);
        frame_index += 1;
    }
    while app
        .world()
        .resource::<host::Summary>()
        .current
        .as_ref()
        .unwrap()
        .backlog
        > Duration::ZERO
    {
        frame(&mut app, Duration::ZERO);
    }
    let summary = app
        .world()
        .resource::<host::Summary>()
        .current
        .clone()
        .unwrap();
    assert_eq!(summary.tick, 12_000);
    assert_eq!(summary.time_ms, 192_000);
    assert_eq!(summary.backlog, Duration::ZERO);
    let audit = app.world().resource::<Audit>();
    let hashes = audit.tick_hashes.clone().unwrap();
    assert_eq!(hashes.len(), 12_000);
    assert!(audit.replacements > 0, "replay requires actual replacement");
    println!(
        "C08 frames={} ticks={} replacements={} last={:02x?}",
        summary.frame,
        summary.tick,
        audit.replacements,
        hashes.last().unwrap()
    );
    (hashes, summary, audit.replacements)
}

#[test]
fn c08_twelve_thousand_real_app_ticks_match_across_all_frame_chunks() {
    let one = run_replay(&[Duration::from_millis(16)]);
    for deltas in [
        vec![Duration::from_millis(64)],
        vec![Duration::from_millis(128)],
        vec![
            Duration::ZERO,
            Duration::from_micros(500),
            Duration::from_micros(15_500),
            Duration::from_millis(160),
            Duration::ZERO,
            Duration::from_millis(64),
        ],
    ] {
        let other = run_replay(&deltas);
        assert_eq!(one.0.len(), other.0.len());
        for (tick, (expected, actual)) in one.0.iter().zip(&other.0).enumerate() {
            assert_eq!(
                actual,
                expected,
                "complete per-tick replay mismatch at {}",
                tick + 1
            );
        }
        assert_eq!(one.1.tick, other.1.tick);
        assert_eq!(one.1.time_ms, other.1.time_ms);
        assert_eq!(one.1.backlog, other.1.backlog);
        assert_eq!(one.2, other.2);
    }
}

fn soak(target: usize) {
    let mut app = app(target, 0);
    add_audit(&mut app, false);
    while app.world().resource::<Audit>().replacements < 50
        && app
            .world()
            .resource::<LaneFlowSession>()
            .world()
            .tick_index()
            < 50_000
    {
        frame(&mut app, Duration::from_millis(128));
    }
    let audit = app.world().resource::<Audit>();
    assert!(
        audit.replacements >= 50,
        "incomplete soak cannot pass on a tick prefix"
    );
    println!(
        "C09 target={target} ticks={} replacements={} capacities={:?}",
        app.world()
            .resource::<LaneFlowSession>()
            .world()
            .tick_index(),
        audit.replacements,
        audit.capacities
    );
}

#[test]
fn c09_fifty_vehicles_recycle_without_growing_population_or_proxies() {
    soak(50);
}

#[test]
fn c09_two_hundred_vehicles_recycle_without_growing_population_or_proxies() {
    soak(200);
}

#[test]
fn c06_missing_time_stops_real_outer_frame_without_losing_existing_backlog() {
    let mut app = app(50, 0);
    frame(&mut app, Duration::from_millis(160));
    let before = states(app.world());
    let counts = app.world().resource::<host::Population>().0.counts();
    let tick = app
        .world()
        .resource::<LaneFlowSession>()
        .world()
        .tick_index();
    app.world_mut().remove_resource::<Time>();
    app.world_mut().run_schedule(LaneFlowOuterFrame);
    host::collect_adapter_error(app.world_mut());
    host::exit_on_failure(app.world_mut());
    let session = app.world().resource::<LaneFlowSession>();
    assert_eq!(
        session.last_error(),
        Some(&LaneFlowAdapterError::MissingTimeResource)
    );
    assert_eq!(session.world().tick_index(), tick);
    assert_eq!(session.frame_report().steps_run(), 0);
    assert_eq!(session.frame_report().backlog(), Duration::from_millis(32));
    assert_eq!(states(app.world()), before);
    assert_eq!(
        app.world().resource::<host::Population>().0.counts(),
        counts
    );
    assert!(app.world().resource::<host::Health>().exit_sent);
    assert!(
        app.world()
            .resource::<Messages<AppExit>>()
            .iter_current_update_messages()
            .any(|exit| matches!(exit, AppExit::Error(_)))
    );
    assert!(app.world().resource::<host::Summary>().current.is_none());
}

#[test]
fn c06_duplicate_observe_stops_remaining_catch_up_after_committed_tick() {
    let mut app = app(50, 0);
    app.add_systems(
        LaneFlowFixed,
        (|world: &mut World| host::observe(world))
            .after(host::observe)
            .in_set(LaneFlowFixedSet::Observe),
    );
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        128,
    )));
    app.update();
    let session = app.world().resource::<LaneFlowSession>();
    assert_eq!(session.world().tick_index(), 1);
    assert_eq!(session.frame_report().steps_run(), 1);
    assert_eq!(session.frame_report().backlog(), Duration::from_millis(112));
    let health = app.world().resource::<host::Health>();
    assert_eq!(health.failure.as_ref().unwrap().stage, host::Stage::Observe);
    assert!(
        health
            .failure
            .as_ref()
            .unwrap()
            .detail
            .contains("NonMonotonicStep")
    );
    assert!(health.exit_sent);
    assert!(app.world().resource::<host::Summary>().current.is_none());
}

#[derive(Resource)]
struct HoldLifecycle(bool);

fn pending_app(minimum: usize) -> App {
    let mut app = app(50, 0);
    app.insert_resource(HoldLifecycle(true)).configure_sets(
        LaneFlowFixed,
        LaneFlowFixedSet::Lifecycle.run_if(|hold: bevy_ecs::system::Res<HoldLifecycle>| !hold.0),
    );
    for _ in 0..6_250 {
        frame(&mut app, Duration::from_millis(128));
        if app
            .world()
            .resource::<host::Population>()
            .0
            .counts()
            .pending
            >= minimum
        {
            break;
        }
    }
    assert!(
        app.world()
            .resource::<host::Population>()
            .0
            .counts()
            .pending
            >= minimum,
        "bounded fixture must complete enough routes"
    );
    app.world_mut().resource_mut::<HoldLifecycle>().0 = false;
    app
}

fn pending_inputs(world: &World) -> Vec<(VehicleHandle, VehicleSpawnInput)> {
    let population = &world.resource::<host::Population>().0;
    let runtime = world.resource::<LaneFlowSession>().world();
    population
        .pending_vehicles()
        .into_iter()
        .map(|old| (old, population.pending_spawn_input(runtime, old).unwrap()))
        .collect()
}

#[test]
fn c05_real_overlapping_entries_keep_the_frozen_plan_and_allow_other_replacements() {
    let mut app = pending_app(20);
    let before = pending_inputs(app.world());
    let rng = app.world().resource::<host::Population>().0.rng_state();
    let mut calls = Vec::new();
    let mut blocked = Vec::new();
    let mut replaced = 0;
    host::apply_with(app.world_mut(), |world, old, input| {
        assert!(
            calls.iter().all(|candidate| *candidate != old),
            "same boundary retry"
        );
        calls.push(old);
        assert_eq!(
            before
                .iter()
                .find(|(vehicle, _)| *vehicle == old)
                .unwrap()
                .1,
            input
        );
        let entity = world
            .resource::<LaneFlowSession>()
            .vehicle_entity(old)
            .unwrap();
        let transform = *world.get::<Transform>(entity).unwrap();
        let cursor = world.resource::<LaneFlowSession>().world().command_cursor();
        let result = replace_completed_vehicle(world, old, input).unwrap();
        match result {
            LaneFlowVehicleReplaceOutcome::Blocked(_) => {
                blocked.push((old, entity, transform));
                assert_eq!(
                    world.resource::<LaneFlowSession>().world().command_cursor(),
                    cursor
                );
            }
            LaneFlowVehicleReplaceOutcome::Replaced(_) => replaced += 1,
            _ => {}
        }
        Ok(result)
    });
    assert!(app.world().resource::<host::Health>().failure.is_none());
    assert_eq!(calls.len(), before.len());
    assert!(
        !blocked.is_empty(),
        "fixture must cause actual Runtime entry overlap"
    );
    assert!(replaced > 0, "one blocked plan must not stop other plans");
    assert_eq!(
        app.world().resource::<host::Population>().0.rng_state(),
        rng
    );
    for &(old, entity, transform) in &blocked {
        assert_eq!(
            app.world()
                .resource::<LaneFlowSession>()
                .vehicle_entity(old),
            Some(entity)
        );
        assert_eq!(*app.world().get::<Transform>(entity).unwrap(), transform);
        assert_eq!(
            pending_inputs(app.world())
                .iter()
                .find(|(vehicle, _)| *vehicle == old)
                .unwrap()
                .1,
            before
                .iter()
                .find(|(vehicle, _)| *vehicle == old)
                .unwrap()
                .1
        );
    }
    for _ in 0..12_000 {
        for &(old, _, _) in &blocked {
            if let Some((_, plan)) = pending_inputs(app.world())
                .iter()
                .find(|(vehicle, _)| *vehicle == old)
            {
                assert_eq!(
                    *plan,
                    before
                        .iter()
                        .find(|(vehicle, _)| *vehicle == old)
                        .unwrap()
                        .1
                );
            }
        }
        if blocked.iter().all(|(old, _, _)| {
            app.world()
                .resource::<LaneFlowSession>()
                .world()
                .vehicle(*old)
                .is_none()
        }) {
            break;
        }
        frame(&mut app, Duration::from_millis(16));
    }
    assert!(
        blocked.iter().all(|(old, _, _)| app
            .world()
            .resource::<LaneFlowSession>()
            .world()
            .vehicle(*old)
            .is_none()),
        "blocked plans must eventually replace"
    );
    assert_eq!(
        app.world().resource::<host::Population>().0.counts().target,
        50
    );
}

#[test]
fn c05_each_adapter_retryable_keeps_identity_and_does_not_starve_other_plans() {
    for kind in 0..3 {
        let mut app = pending_app(3);
        let before = pending_inputs(app.world());
        let (old, frozen) = before[0];
        let rng = app.world().resource::<host::Population>().0.rng_state();
        let entity = app
            .world()
            .resource::<LaneFlowSession>()
            .vehicle_entity(old)
            .unwrap();
        let transform = *app.world().get::<Transform>(entity).unwrap();
        let mut attempted = Vec::new();
        let mut replaced = 0;
        host::apply_with(app.world_mut(), |world, vehicle, input| {
            assert!(!attempted.contains(&vehicle));
            attempted.push(vehicle);
            if vehicle == old {
                assert_eq!(input, frozen);
                let error = match kind {
                    0 => laneflow_runtime::ReplaceError::StopConstraintUnsatisfiable,
                    1 => laneflow_runtime::ReplaceError::UnsafeLeader { leader: old },
                    _ => laneflow_runtime::ReplaceError::UnsafeFollower { follower: old },
                };
                Ok(LaneFlowVehicleReplaceOutcome::Retryable(error))
            } else {
                let result = replace_completed_vehicle(world, vehicle, input).unwrap();
                if matches!(result, LaneFlowVehicleReplaceOutcome::Replaced(_)) {
                    replaced += 1;
                }
                Ok(result)
            }
        });
        assert_eq!(attempted.len(), before.len());
        assert!(replaced > 0);
        assert!(app.world().resource::<host::Health>().failure.is_none());
        assert_eq!(
            app.world().resource::<host::Population>().0.rng_state(),
            rng
        );
        assert_eq!(
            app.world()
                .resource::<LaneFlowSession>()
                .vehicle_entity(old),
            Some(entity)
        );
        assert_eq!(*app.world().get::<Transform>(entity).unwrap(), transform);
        assert_eq!(
            pending_inputs(app.world())
                .iter()
                .find(|(vehicle, _)| *vehicle == old)
                .unwrap()
                .1,
            frozen
        );
        for _ in 0..12_000 {
            if app
                .world()
                .resource::<LaneFlowSession>()
                .world()
                .vehicle(old)
                .is_none()
            {
                break;
            }
            assert_eq!(
                pending_inputs(app.world())
                    .iter()
                    .find(|(vehicle, _)| *vehicle == old)
                    .unwrap()
                    .1,
                frozen
            );
            frame(&mut app, Duration::from_millis(16));
        }
        assert!(
            app.world()
                .resource::<LaneFlowSession>()
                .world()
                .vehicle(old)
                .is_none()
        );
        assert!(app.world().get::<host::VehicleProxy>(entity).is_some());
    }
}

#[derive(Resource, Default)]
struct PriorCommit(Option<laneflow_runtime::VehicleReplaceRecord>);

fn fail_after_one_committed_replacement(world: &mut World) {
    host::apply_with(world, |world, old, input| {
        if world.resource::<PriorCommit>().0.is_some() {
            return Err("C06 后续 host 命令失败".into());
        }
        let result = replace_completed_vehicle(world, old, input).unwrap();
        if let LaneFlowVehicleReplaceOutcome::Replaced(record) = result {
            world.resource_mut::<PriorCommit>().0 = Some(laneflow_runtime::VehicleReplaceRecord {
                old: record.old,
                new: record.new,
            });
        }
        Ok(result)
    });
}

#[test]
fn c06_lifecycle_failure_preserves_prior_commands_and_stops_same_frame_step() {
    let mut app = pending_app(20);
    let tick = app
        .world()
        .resource::<LaneFlowSession>()
        .world()
        .tick_index();
    app.init_resource::<PriorCommit>().add_systems(
        LaneFlowFixed,
        fail_after_one_committed_replacement
            .before(host::lifecycle)
            .in_set(LaneFlowFixedSet::Lifecycle),
    );
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        128,
    )));
    app.update();
    let committed = app
        .world()
        .resource::<PriorCommit>()
        .0
        .expect("fixture must commit the first command");
    let session = app.world().resource::<LaneFlowSession>();
    assert_eq!(session.world().tick_index(), tick);
    assert_eq!(session.frame_report().steps_run(), 0);
    assert_eq!(session.frame_report().backlog(), Duration::from_millis(128));
    assert!(session.world().vehicle(committed.old).is_none());
    assert!(session.world().vehicle(committed.new).is_some());
    let health = app.world().resource::<host::Health>();
    assert_eq!(
        health.failure.as_ref().unwrap().stage,
        host::Stage::Lifecycle
    );
    assert!(
        health
            .failure
            .as_ref()
            .unwrap()
            .detail
            .contains("后续 host 命令失败")
    );
    assert!(health.exit_sent);
    assert!(app.world().resource::<host::Summary>().current.is_none());
}

#[test]
fn c07_wrong_result_after_commit_is_fatal_and_never_retries_stale_old() {
    for fault in 0..3 {
        let mut app = pending_app(3);
        let other = app
            .world()
            .resource::<host::Population>()
            .0
            .logical_vehicle(49)
            .unwrap();
        let wrong_entity = app.world().resource::<host::Proxies>().root;
        let mut committed = None;
        host::apply_with(app.world_mut(), |world, old, input| {
            let mut result = replace_completed_vehicle(world, old, input).unwrap();
            if let LaneFlowVehicleReplaceOutcome::Replaced(ref mut record) = result {
                committed = Some(*record);
                match fault {
                    0 => record.old = other,
                    1 => record.new = other,
                    _ => record.entity = Some(wrong_entity),
                }
            }
            Ok(result)
        });
        let committed = committed.expect("fixture must commit before outcome corruption");
        assert!(
            app.world()
                .resource::<LaneFlowSession>()
                .world()
                .vehicle(committed.old)
                .is_none()
        );
        assert!(
            app.world()
                .resource::<LaneFlowSession>()
                .world()
                .vehicle(committed.new)
                .is_some()
        );
        assert_eq!(
            app.world()
                .resource::<host::Health>()
                .failure
                .as_ref()
                .unwrap()
                .stage,
            host::Stage::Lifecycle
        );
        let cursor = app
            .world()
            .resource::<LaneFlowSession>()
            .world()
            .command_cursor();
        host::apply_with(app.world_mut(), |_, _, _| {
            panic!("failed host must never retry")
        });
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            128,
        )));
        app.update();
        assert_eq!(
            app.world()
                .resource::<LaneFlowSession>()
                .world()
                .command_cursor(),
            cursor
        );
        assert!(app.world().resource::<host::Health>().exit_sent);
    }
}

#[test]
fn c07_context_precheck_and_single_host_assembly_reject_miswiring() {
    let mut app = app(50, 0);
    let before = states(app.world());
    let revision = app
        .world()
        .resource::<LaneFlowSession>()
        .world()
        .revision()
        .network_revision();
    let error = app
        .world_mut()
        .resource_mut::<host::Population>()
        .0
        .apply_pending(
            revision,
            laneflow_runtime::WorldPolicySelection::NotRequired,
            |_, _| -> Result<_, String> { panic!("context mismatch must reject before callback") },
        )
        .unwrap_err();
    assert!(matches!(
        error,
        laneflow_scenario::signalized_corridor::CorridorReplaceApplyError::Policy(_)
    ));
    assert_eq!(states(app.world()), before);
    let foreign = laneflow_static_contract::NetworkRevisionId::from_digest(
        laneflow_static_contract::Sha256Digest::from_bytes([0; 32]),
    );
    let policy = app
        .world()
        .resource::<LaneFlowSession>()
        .world()
        .policy_selection();
    let error = app
        .world_mut()
        .resource_mut::<host::Population>()
        .0
        .apply_pending(foreign, policy, |_, _| -> Result<_, String> {
            panic!("foreign revision callback")
        })
        .unwrap_err();
    assert!(matches!(
        error,
        laneflow_scenario::signalized_corridor::CorridorReplaceApplyError::Policy(_)
    ));
    let second = scene::build(CorridorPopulationConfig::try_new(50, 73).unwrap()).unwrap();
    assert!(host::install(&mut app, second).is_err());
    assert_eq!(states(app.world()), before);
}

#[test]
fn c07_skipped_observe_is_rejected_at_the_next_committed_tick() {
    let mut app = app(50, 0);
    app.configure_sets(
        LaneFlowFixed,
        LaneFlowFixedSet::Observe.run_if(|session: bevy_ecs::system::Res<LaneFlowSession>| {
            session.world().tick_index() != 1
        }),
    );
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        128,
    )));
    app.update();
    assert_eq!(
        app.world()
            .resource::<LaneFlowSession>()
            .world()
            .tick_index(),
        2
    );
    assert_eq!(
        app.world()
            .resource::<LaneFlowSession>()
            .frame_report()
            .backlog(),
        Duration::from_millis(96)
    );
    assert!(
        app.world()
            .resource::<host::Health>()
            .failure
            .as_ref()
            .unwrap()
            .detail
            .contains("NonMonotonicStep")
    );
}

#[test]
fn c06_lifecycle_policy_error_stops_before_step_in_the_same_outer_frame() {
    let mut app = app(50, 0);
    let before = states(app.world());
    app.add_systems(
        LaneFlowFixed,
        (|world: &mut World| {
            let revision = world
                .resource::<LaneFlowSession>()
                .world()
                .revision()
                .network_revision();
            let error = world
                .resource_mut::<host::Population>()
                .0
                .apply_pending(
                    revision,
                    laneflow_runtime::WorldPolicySelection::NotRequired,
                    |_, _| -> Result<_, String> { panic!("policy precheck callback") },
                )
                .unwrap_err();
            host::fail(world, host::Stage::Lifecycle, error);
        })
        .before(host::lifecycle)
        .in_set(LaneFlowFixedSet::Lifecycle),
    );
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        128,
    )));
    app.update();
    assert_eq!(states(app.world()), before);
    let session = app.world().resource::<LaneFlowSession>();
    assert_eq!(session.world().tick_index(), 0);
    assert_eq!(session.frame_report().backlog(), Duration::from_millis(128));
    assert_eq!(
        app.world()
            .resource::<host::Health>()
            .failure
            .as_ref()
            .unwrap()
            .stage,
        host::Stage::Lifecycle
    );
    assert!(app.world().resource::<host::Health>().exit_sent);
}

#[test]
fn c07_fatal_motion_error_cannot_be_laundered_into_retryable() {
    let mut app = pending_app(1);
    let before = states(app.world());
    host::apply_with(app.world_mut(), |_, _, _| {
        Ok(LaneFlowVehicleReplaceOutcome::Retryable(
            laneflow_runtime::ReplaceError::DownstreamSpeedUnsatisfiable,
        ))
    });
    assert_eq!(states(app.world()), before);
    assert!(
        app.world()
            .resource::<host::Health>()
            .failure
            .as_ref()
            .unwrap()
            .detail
            .contains("DownstreamSpeedUnsatisfiable")
    );
}

#[test]
fn c11_every_lamp_follows_committed_groups_across_full_cycle_and_wrap() {
    use laneflow_static_contract::{SignalControllerOrdinal, SignalGroupOrdinal};
    let mut app = app(50, 0);
    let geometry = app.world().resource::<scene::Geometry>().clone();
    assert_eq!(
        geometry
            .lamps
            .iter()
            .map(|lamp| lamp.stop)
            .collect::<HashSet<_>>()
            .len(),
        geometry.lamps.len()
    );
    assert!(scene::common_group([]).is_err());
    assert!(scene::common_group([None]).is_err());
    assert!(scene::common_group([Some(SignalGroupOrdinal::from_raw(0)), None]).is_err());
    assert!(
        scene::common_group([
            Some(SignalGroupOrdinal::from_raw(0)),
            Some(SignalGroupOrdinal::from_raw(1))
        ])
        .is_err()
    );
    let initial = app
        .world()
        .resource::<host::Summary>()
        .current
        .as_ref()
        .unwrap()
        .signals
        .clone();
    let mut visited = HashSet::new();
    for tick in 1..=5_255 {
        frame(&mut app, Duration::from_millis(16));
        let summary = app
            .world()
            .resource::<host::Summary>()
            .current
            .as_ref()
            .unwrap();
        let session = app.world().resource::<LaneFlowSession>();
        assert_eq!(
            summary.signals,
            session.world().committed_signal_groups().as_slice()
        );
        let revision = session.world().revision();
        let relations = revision.traffic().relations();
        for controller_index in 0..2 {
            let controller = relations
                .signal_controller(SignalControllerOrdinal::from_raw(controller_index))
                .unwrap();
            assert_eq!(controller.cycle_ms(), 84_064);
            assert_eq!(controller.phases().len(), 12);
            let position = (summary.time_ms + controller.offset_ms()) % controller.cycle_ms();
            let mut cumulative = 0;
            for &phase in controller.phases() {
                let view = relations.signal_phase(phase).unwrap();
                cumulative += view.duration_ms();
                if position < cumulative {
                    visited.insert(phase);
                    for (group, aspect) in view.states() {
                        assert_eq!(
                            summary
                                .signals
                                .iter()
                                .find(|(candidate, _)| *candidate == group)
                                .unwrap()
                                .1,
                            aspect
                        );
                    }
                    break;
                }
            }
        }
        for lamp in &geometry.lamps {
            assert!(
                summary
                    .signals
                    .iter()
                    .any(|(group, _)| *group == lamp.group)
            );
        }
        if tick == 5_254 {
            assert_eq!(summary.signals, initial);
        }
    }
    assert_eq!(visited.len(), 24, "both controllers must visit every phase");
}

fn assert_hud_matches_sources(app: &App) {
    let summary = app
        .world()
        .resource::<host::Summary>()
        .current
        .as_ref()
        .unwrap();
    let counts = app.world().resource::<host::Population>().0.counts();
    let report = app.world().resource::<LaneFlowSession>().frame_report();
    assert_eq!(summary.counts, counts);
    assert_eq!(summary.steps, report.steps_run());
    assert_eq!(summary.backlog, report.backlog());
    assert_eq!(summary.limited, report.catch_up_limit_reached());
    let hud = summary.hud();
    assert!(hud.is_ascii());
    assert!(hud.contains(&format!(
        "vehicles {}   running {}   pending {}",
        counts.target, counts.running, counts.pending
    )));
    assert!(hud.contains(&format!(
        "steps {}   backlog_ms {:.3}",
        report.steps_run(),
        report.backlog().as_secs_f64() * 1_000.0
    )));
}

#[test]
fn c12_zero_regular_limited_pending_and_recycled_frames_share_one_summary() {
    let mut app = app(100, 0);
    assert_hud_matches_sources(&app);
    frame(&mut app, Duration::from_nanos(123_456));
    assert_hud_matches_sources(&app);
    let summary = app
        .world()
        .resource::<host::Summary>()
        .current
        .as_ref()
        .unwrap();
    assert_eq!(summary.backlog.as_nanos(), 123_456);
    assert!(summary.hud().contains("backlog_ms 0.123"));
    frame(&mut app, Duration::from_millis(16));
    assert_hud_matches_sources(&app);
    frame(&mut app, Duration::from_millis(160));
    assert_hud_matches_sources(&app);
    let mut pending = pending_app(3);
    assert_hud_matches_sources(&pending);
    assert!(
        pending
            .world()
            .resource::<host::Summary>()
            .current
            .as_ref()
            .unwrap()
            .counts
            .pending
            > 0
    );
    frame(&mut pending, Duration::from_millis(16));
    assert_hud_matches_sources(&pending);
    assert!(
        !pending
            .world()
            .resource::<host::BoundaryTrace>()
            .replacements
            .is_empty()
    );
}

#[test]
fn c13_independent_headless_capture_reference_uses_the_shared_delta_script() {
    let target = std::env::var("CORRIDOR_TRAFFIC_CAPTURE_TICK")
        .ok()
        .map(|value| value.parse::<u64>().unwrap())
        .unwrap_or(replay::DEFAULT_CAPTURE_TICK);
    let mut app = App::new();
    app.add_plugins((TimePlugin, TransformPlugin));
    host::install(
        &mut app,
        scene::build(CorridorPopulationConfig::default()).unwrap(),
    )
    .unwrap();
    replay::install(&mut app, replay::ReplayClock::new(target).unwrap());
    for _ in 0..50_000 {
        app.update();
        assert!(app.world().resource::<host::Health>().failure.is_none());
        let summary = app
            .world()
            .resource::<host::Summary>()
            .current
            .as_ref()
            .unwrap();
        assert_eq!(
            summary.frame,
            app.world().resource::<replay::ReplayClock>().frame
        );
        if app.world().resource::<replay::ReplayClock>().ready(summary) {
            let evidence = replay::evidence(summary);
            let parsed: toml::Value = toml::from_str(&evidence).unwrap();
            assert_eq!(parsed["tick"].as_integer(), Some(target as i64));
            assert_eq!(parsed["hud"].as_str(), Some(summary.hud().as_str()));
            if let Some(path) = std::env::var_os("CORRIDOR_TRAFFIC_REFERENCE") {
                std::fs::write(path, &evidence).unwrap();
            }
            println!(
                "C13 headless frame={} tick={} running={} pending={}",
                summary.frame, summary.tick, summary.counts.running, summary.counts.pending
            );
            return;
        }
    }
    panic!("capture script did not finish within bounded frames");
}
