use std::{num::NonZeroU32, time::Duration};

use bevy_app::App;
use bevy_ecs::{
    hierarchy::ChildOf,
    resource::Resource,
    schedule::IntoScheduleConfigs,
    system::{Res, ResMut},
    world::World,
};
use bevy_time::{TimePlugin, TimeUpdateStrategy};
use bevy_transform::{TransformPlugin, components::Transform};
use laneflow_bevy::{
    LaneFlowAdapterError, LaneFlowFixed, LaneFlowFixedSet, LaneFlowFramePlacement, LaneFlowPlugin,
    LaneFlowSession, LaneFlowSessionConfig, LaneFlowVehicleReplaceOutcome,
    replace_completed_vehicle,
};
use laneflow_core::{
    CoreWorld, EdgeLength, EdgeProgress, IidmProfileSpec, InitialTrafficData, LaneEdge, LaneGraph,
    Route, Speed, SpeedLimit, VehicleHandle, VehicleProfile, VehicleProfileHandle,
    VehicleProfileRegistry, VehicleReplaceExternalId, VehicleReplaceInput, VehicleSpawnInput,
    VehicleStatus,
};
use laneflow_spatial::{
    CanonicalFrameId, CanonicalPoint3F32, FramePlacementToken, SpatialEdgeInput, SpatialRegistry,
};

struct ReplaceFixture {
    session: LaneFlowSession,
    old: VehicleHandle,
    profile: VehicleProfileHandle,
}

fn progress(value: f64) -> EdgeProgress {
    EdgeProgress::try_new(value).expect("valid progress")
}

fn profile() -> VehicleProfile {
    VehicleProfile::try_new_iidm(
        "profile",
        IidmProfileSpec {
            length: 4.5,
            desired_speed: 13.9,
            min_gap: 2.0,
            time_headway: 1.5,
            max_acceleration: 1.4,
            comfortable_deceleration: 2.0,
            emergency_deceleration: 4.0,
        },
    )
    .expect("valid profile")
}

fn fixture(old_status: VehicleStatus, with_blocker: bool, pose_capacity: usize) -> ReplaceFixture {
    let graph = LaneGraph::try_new([
        LaneEdge::new(
            "old-edge",
            EdgeLength::try_new(100.0).expect("old length"),
            SpeedLimit::try_new(20.0).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
        LaneEdge::new(
            "target-edge",
            EdgeLength::try_new(100.0).expect("target length"),
            SpeedLimit::try_new(20.0).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
    ])
    .expect("valid graph");
    let old_edge = graph.edge_handle("old-edge").expect("old edge");
    let target_edge = graph.edge_handle("target-edge").expect("target edge");
    let old_points = [
        CanonicalPoint3F32::try_new(0.0, 0.0, -10.0).expect("old start"),
        CanonicalPoint3F32::try_new(100.0, 0.0, -10.0).expect("old end"),
    ];
    let target_points = [
        CanonicalPoint3F32::try_new(0.0, 0.0, 10.0).expect("target start"),
        CanonicalPoint3F32::try_new(100.0, 0.0, 10.0).expect("target end"),
    ];
    let spatial = SpatialRegistry::try_new(
        &graph,
        CanonicalFrameId::try_new("test:lifecycle").expect("frame"),
        [
            SpatialEdgeInput::new(old_edge, &old_points),
            SpatialEdgeInput::new(target_edge, &target_points),
        ],
    )
    .expect("spatial registry");
    let profiles = VehicleProfileRegistry::try_new([profile()]).expect("valid profiles");
    let profile = profiles.profile_handle("profile").expect("profile handle");
    let traffic = InitialTrafficData::try_new(
        graph,
        [
            Route::try_new("old-route", ["old-edge"]).expect("old route"),
            Route::try_new("target-route", ["target-edge"]).expect("target route"),
        ],
        profiles,
    )
    .expect("valid traffic");
    let old = VehicleSpawnInput::new(
        "old",
        profile,
        "old-route",
        0,
        if old_status == VehicleStatus::Completed {
            progress(100.0)
        } else {
            progress(50.0)
        },
        Speed::ZERO,
        old_status,
    );
    let mut vehicles = vec![old];
    if with_blocker {
        vehicles.push(VehicleSpawnInput::active(
            "blocker",
            profile,
            "target-route",
            0,
            progress(12.0),
            Speed::ZERO,
        ));
    }
    let core = CoreWorld::with_traffic_data(20, traffic, vehicles).expect("valid world");
    let old = core.vehicle_handle("old").expect("old handle");
    let config = LaneFlowSessionConfig::new(NonZeroU32::new(4).expect("non-zero"));
    ReplaceFixture {
        session: LaneFlowSession::with_pose_capacity(core, spatial, config, pose_capacity),
        old,
        profile,
    }
}

fn input(
    session: &LaneFlowSession,
    profile: VehicleProfileHandle,
    edge_progress: f64,
) -> VehicleReplaceInput {
    VehicleReplaceInput::new(
        VehicleReplaceExternalId::Preserve,
        profile,
        session
            .core()
            .route_handle("target-route")
            .expect("target route"),
        0,
        progress(edge_progress),
        Speed::ZERO,
    )
}

#[test]
fn bound_success_reuses_entity_rotates_mapping_and_retains_transform() {
    let ReplaceFixture {
        mut session,
        old,
        profile,
    } = fixture(VehicleStatus::Completed, false, 1);
    let replacement = input(&session, profile, 10.0);
    let mut world = World::new();
    let original = Transform::from_xyz(3.0, 4.0, 5.0);
    let entity = world.spawn(original).id();
    session
        .bind_vehicle_entity(old, entity)
        .expect("bind old vehicle");
    world.insert_resource(session);

    let LaneFlowVehicleReplaceOutcome::Replaced(record) =
        replace_completed_vehicle(&mut world, old, &replacement).expect("replacement")
    else {
        panic!("replacement must succeed")
    };

    let session = world.resource::<LaneFlowSession>();
    assert_eq!(record.old, old);
    assert_ne!(record.new, old);
    assert_eq!(record.entity, Some(entity));
    assert_eq!(session.core().vehicle(old), None);
    assert_eq!(session.vehicle_entities().len(), 1);
    assert_eq!(session.vehicle_entities().entity(old), None);
    assert_eq!(session.vehicle_entities().entity(record.new), Some(entity));
    assert_eq!(session.vehicle_entities().vehicle(entity), Some(record.new));
    assert_eq!(world.get::<Transform>(entity), Some(&original));
}

#[test]
fn next_normal_presentation_updates_the_reused_entity_from_the_new_handle() {
    let ReplaceFixture {
        mut session,
        old,
        profile,
    } = fixture(VehicleStatus::Completed, false, 1);
    let replacement = input(&session, profile, 10.0);
    let mut app = App::new();
    app.add_plugins((TimePlugin, TransformPlugin, LaneFlowPlugin));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    let root = app.world_mut().spawn(Transform::IDENTITY).id();
    let original = Transform::from_xyz(3.0, 4.0, 5.0);
    let entity = app.world_mut().spawn((original, ChildOf(root))).id();
    session
        .bind_vehicle_entity(old, entity)
        .expect("bind old vehicle");
    session
        .set_frame_placement(LaneFlowFramePlacement::new(
            root,
            FramePlacementToken::new(1),
        ))
        .expect("frame placement");
    app.insert_resource(session);

    let LaneFlowVehicleReplaceOutcome::Replaced(record) =
        replace_completed_vehicle(app.world_mut(), old, &replacement).expect("replacement")
    else {
        panic!("replacement must succeed")
    };
    assert_eq!(app.world().get::<Transform>(entity), Some(&original));

    app.update();

    let session = app.world().resource::<LaneFlowSession>();
    assert!(session.last_error().is_none());
    assert_eq!(session.vehicle_entities().entity(record.new), Some(entity));
    let presented = app
        .world()
        .get::<Transform>(entity)
        .expect("proxy Transform");
    assert_eq!(presented.translation.x, 10.0);
    assert_eq!(presented.translation.y, 0.0);
    assert_eq!(presented.translation.z, 10.0);
}

#[test]
fn unbound_success_stays_unbound() {
    let ReplaceFixture {
        session,
        old,
        profile,
    } = fixture(VehicleStatus::Completed, false, 0);
    let replacement = input(&session, profile, 10.0);
    let mut world = World::new();
    world.insert_resource(session);

    let LaneFlowVehicleReplaceOutcome::Replaced(record) =
        replace_completed_vehicle(&mut world, old, &replacement).expect("replacement")
    else {
        panic!("replacement must succeed")
    };

    let session = world.resource::<LaneFlowSession>();
    assert_eq!(record.entity, None);
    assert!(session.vehicle_entities().is_empty());
    assert!(session.core().vehicle(record.new).is_some());
}

#[test]
fn blocked_is_retryable_and_leaves_core_mapping_transform_and_error_unchanged() {
    let ReplaceFixture {
        mut session,
        old,
        profile,
    } = fixture(VehicleStatus::Completed, true, 1);
    let replacement = input(&session, profile, 10.0);
    let blocker = session
        .core()
        .vehicle_handle("blocker")
        .expect("blocker handle");
    let mut world = World::new();
    let original = Transform::from_xyz(1.0, 2.0, 3.0);
    let entity = world.spawn(original).id();
    session
        .bind_vehicle_entity(old, entity)
        .expect("bind old vehicle");
    world.insert_resource(session);

    let LaneFlowVehicleReplaceOutcome::Blocked(block) =
        replace_completed_vehicle(&mut world, old, &replacement).expect("typed block")
    else {
        panic!("overlap must block")
    };

    let session = world.resource::<LaneFlowSession>();
    assert_eq!(block.old, old);
    assert_eq!(block.blocker, blocker);
    assert!(session.core().vehicle(old).is_some());
    assert_eq!(session.vehicle_entities().entity(old), Some(entity));
    assert!(session.last_error().is_none());
    assert_eq!(world.get::<Transform>(entity), Some(&original));
}

#[test]
fn blocked_command_does_not_prevent_a_later_command_at_the_same_boundary() {
    let graph = LaneGraph::try_new([
        LaneEdge::new(
            "old-edge",
            EdgeLength::try_new(100.0).expect("old length"),
            SpeedLimit::try_new(20.0).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
        LaneEdge::new(
            "target-a",
            EdgeLength::try_new(100.0).expect("target length"),
            SpeedLimit::try_new(20.0).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
        LaneEdge::new(
            "target-b",
            EdgeLength::try_new(100.0).expect("target length"),
            SpeedLimit::try_new(20.0).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
    ])
    .expect("valid graph");
    let profiles = VehicleProfileRegistry::try_new([profile()]).expect("valid profiles");
    let profile = profiles.profile_handle("profile").expect("profile handle");
    let traffic = InitialTrafficData::try_new(
        graph,
        [
            Route::try_new("old-route", ["old-edge"]).expect("old route"),
            Route::try_new("route-a", ["target-a"]).expect("route a"),
            Route::try_new("route-b", ["target-b"]).expect("route b"),
        ],
        profiles,
    )
    .expect("valid traffic");
    let core = CoreWorld::with_traffic_data(
        20,
        traffic,
        vec![
            VehicleSpawnInput::completed("old-a", profile, "old-route", 0, progress(100.0)),
            VehicleSpawnInput::completed("old-b", profile, "old-route", 0, progress(100.0)),
            VehicleSpawnInput::active(
                "blocker",
                profile,
                "route-a",
                0,
                progress(12.0),
                Speed::ZERO,
            ),
        ],
    )
    .expect("valid world");
    let old_a = core.vehicle_handle("old-a").expect("old a");
    let old_b = core.vehicle_handle("old-b").expect("old b");
    let input_a = VehicleReplaceInput::new(
        VehicleReplaceExternalId::Preserve,
        profile,
        core.route_handle("route-a").expect("route a"),
        0,
        progress(10.0),
        Speed::ZERO,
    );
    let input_b = VehicleReplaceInput::new(
        VehicleReplaceExternalId::Preserve,
        profile,
        core.route_handle("route-b").expect("route b"),
        0,
        progress(10.0),
        Speed::ZERO,
    );
    let spatial = SpatialRegistry::try_new(
        &LaneGraph::empty(),
        CanonicalFrameId::try_new("test:multiple-commands").expect("frame"),
        [],
    )
    .expect("empty spatial");
    let config = LaneFlowSessionConfig::new(NonZeroU32::new(1).expect("non-zero"));
    let mut world = World::new();
    world.insert_resource(LaneFlowSession::with_pose_capacity(
        core, spatial, config, 0,
    ));

    assert!(matches!(
        replace_completed_vehicle(&mut world, old_a, &input_a),
        Ok(LaneFlowVehicleReplaceOutcome::Blocked(_))
    ));
    let LaneFlowVehicleReplaceOutcome::Replaced(record) =
        replace_completed_vehicle(&mut world, old_b, &input_b).expect("second command")
    else {
        panic!("independent command must succeed")
    };

    let session = world.resource::<LaneFlowSession>();
    assert!(session.core().vehicle(old_a).is_some());
    assert!(session.core().vehicle(old_b).is_none());
    assert!(session.core().vehicle(record.new).is_some());
    assert!(session.last_error().is_none());
}

#[test]
fn stale_entity_and_core_failure_are_fatal_and_atomic() {
    let ReplaceFixture {
        mut session,
        old,
        profile,
    } = fixture(VehicleStatus::Completed, false, 1);
    let replacement = input(&session, profile, 10.0);
    let mut stale_world = World::new();
    let stale = stale_world.spawn_empty().id();
    session
        .bind_vehicle_entity(old, stale)
        .expect("bind old vehicle");
    stale_world.despawn(stale);
    stale_world.insert_resource(session);

    assert!(matches!(
        replace_completed_vehicle(&mut stale_world, old, &replacement),
        Err(LaneFlowAdapterError::StaleLifecycleEntity { vehicle, entity })
            if vehicle == old && entity == stale
    ));
    let session = stale_world.resource::<LaneFlowSession>();
    assert!(session.core().vehicle(old).is_some());
    assert_eq!(session.vehicle_entities().entity(old), Some(stale));
    assert!(matches!(
        session.last_error(),
        Some(LaneFlowAdapterError::StaleLifecycleEntity { .. })
    ));

    let ReplaceFixture {
        session,
        old,
        profile,
    } = fixture(VehicleStatus::Active, false, 0);
    let replacement = input(&session, profile, 10.0);
    let mut fatal_world = World::new();
    fatal_world.insert_resource(session);
    assert!(matches!(
        replace_completed_vehicle(&mut fatal_world, old, &replacement),
        Err(LaneFlowAdapterError::CoreVehicleReplace { old: failed, .. }) if failed == old
    ));
    let session = fatal_world.resource::<LaneFlowSession>();
    assert!(session.core().vehicle(old).is_some());
    assert!(matches!(
        session.last_error(),
        Some(LaneFlowAdapterError::CoreVehicleReplace { .. })
    ));
}

#[derive(Resource, Default)]
struct FixedTrace(Vec<(&'static str, u64)>);

fn trace_lifecycle(session: Res<'_, LaneFlowSession>, mut trace: ResMut<'_, FixedTrace>) {
    trace.0.push(("lifecycle", session.core().tick_index()));
}

fn trace_observe(session: Res<'_, LaneFlowSession>, mut trace: ResMut<'_, FixedTrace>) {
    trace.0.push(("observe", session.core().tick_index()));
}

#[test]
fn public_sets_repeat_lifecycle_step_observe_for_each_catch_up_step() {
    let ReplaceFixture { session, .. } = fixture(VehicleStatus::Completed, false, 0);
    let mut app = App::new();
    app.add_plugins((TimePlugin, LaneFlowPlugin));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    app.insert_resource(session);
    app.init_resource::<FixedTrace>();
    app.add_systems(
        LaneFlowFixed,
        (
            trace_lifecycle.in_set(LaneFlowFixedSet::Lifecycle),
            trace_observe.in_set(LaneFlowFixedSet::Observe),
        ),
    );
    app.update();

    *app.world_mut().resource_mut::<TimeUpdateStrategy>() =
        TimeUpdateStrategy::ManualDuration(Duration::from_millis(40));
    app.update();

    assert_eq!(
        app.world().resource::<FixedTrace>().0,
        [
            ("lifecycle", 0),
            ("observe", 1),
            ("lifecycle", 1),
            ("observe", 2),
        ]
    );
}

#[derive(Resource)]
struct RecyclePolicy {
    current: VehicleHandle,
    input: VehicleReplaceInput,
    replacements: Vec<VehicleHandle>,
}

fn recycle_at_boundary(world: &mut World) {
    let (old, input) = {
        let policy = world.resource::<RecyclePolicy>();
        (policy.current, policy.input.clone())
    };
    let LaneFlowVehicleReplaceOutcome::Replaced(record) =
        replace_completed_vehicle(world, old, &input).expect("boundary replacement")
    else {
        panic!("end-of-route replacement must succeed")
    };
    let mut policy = world.resource_mut::<RecyclePolicy>();
    policy.current = record.new;
    policy.replacements.push(record.new);
}

fn recycling_app() -> (App, bevy_ecs::entity::Entity, Transform) {
    let ReplaceFixture {
        mut session,
        old,
        profile,
    } = fixture(VehicleStatus::Completed, false, 1);
    let replacement = input(&session, profile, 100.0);
    let mut app = App::new();
    app.add_plugins((TimePlugin, TransformPlugin, LaneFlowPlugin));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    let root = app.world_mut().spawn(Transform::IDENTITY).id();
    let original = Transform::from_xyz(7.0, 8.0, 9.0);
    let entity = app.world_mut().spawn((original, ChildOf(root))).id();
    session
        .bind_vehicle_entity(old, entity)
        .expect("bind old vehicle");
    session
        .set_frame_placement(LaneFlowFramePlacement::new(
            root,
            FramePlacementToken::new(1),
        ))
        .expect("frame placement");
    app.insert_resource(session);
    app.insert_resource(RecyclePolicy {
        current: old,
        input: replacement,
        replacements: Vec::with_capacity(2),
    });
    app.add_systems(
        LaneFlowFixed,
        recycle_at_boundary.in_set(LaneFlowFixedSet::Lifecycle),
    );
    app.update();

    (app, entity, original)
}

#[test]
fn consecutive_catch_up_steps_recycle_the_same_entity_without_immediate_transform_write() {
    let (mut app, entity, original) = recycling_app();

    *app.world_mut().resource_mut::<TimeUpdateStrategy>() =
        TimeUpdateStrategy::ManualDuration(Duration::from_millis(40));
    app.update();

    let policy = app.world().resource::<RecyclePolicy>();
    assert_eq!(policy.replacements.len(), 2);
    let current = policy.current;
    let session = app.world().resource::<LaneFlowSession>();
    assert_eq!(session.core().tick_index(), 2);
    assert_eq!(
        session.core().vehicle(current).unwrap().status,
        VehicleStatus::Completed
    );
    assert_eq!(session.vehicle_entities().len(), 1);
    assert_eq!(session.vehicle_entities().entity(current), Some(entity));
    assert_eq!(session.vehicle_entities().vehicle(entity), Some(current));
    assert_eq!(app.world().get::<Transform>(entity), Some(&original));
}

#[test]
fn recycle_replay_is_independent_of_outer_frame_chunking() {
    let (mut partitioned, partitioned_entity, partitioned_transform) = recycling_app();
    let (mut batched, batched_entity, batched_transform) = recycling_app();

    for delta in [20, 20] {
        *partitioned.world_mut().resource_mut::<TimeUpdateStrategy>() =
            TimeUpdateStrategy::ManualDuration(Duration::from_millis(delta));
        partitioned.update();
    }
    *batched.world_mut().resource_mut::<TimeUpdateStrategy>() =
        TimeUpdateStrategy::ManualDuration(Duration::from_millis(40));
    batched.update();

    let partitioned_policy = partitioned.world().resource::<RecyclePolicy>();
    let batched_policy = batched.world().resource::<RecyclePolicy>();
    assert_eq!(partitioned_policy.current, batched_policy.current);
    assert_eq!(partitioned_policy.replacements, batched_policy.replacements);
    let partitioned_session = partitioned.world().resource::<LaneFlowSession>();
    let batched_session = batched.world().resource::<LaneFlowSession>();
    assert_eq!(partitioned_session.core(), batched_session.core());
    assert_eq!(
        partitioned_session
            .vehicle_entities()
            .entity(partitioned_policy.current),
        Some(partitioned_entity)
    );
    assert_eq!(
        batched_session
            .vehicle_entities()
            .entity(batched_policy.current),
        Some(batched_entity)
    );
    assert_eq!(
        partitioned.world().get::<Transform>(partitioned_entity),
        Some(&partitioned_transform)
    );
    assert_eq!(
        batched.world().get::<Transform>(batched_entity),
        Some(&batched_transform)
    );
}

#[derive(Resource)]
struct FatalPolicy {
    old: VehicleHandle,
    input: VehicleReplaceInput,
    attempted: bool,
}

fn fail_once_at_boundary(world: &mut World) {
    let (old, input, attempted) = {
        let policy = world.resource::<FatalPolicy>();
        (policy.old, policy.input.clone(), policy.attempted)
    };
    if attempted {
        return;
    }
    world.resource_mut::<FatalPolicy>().attempted = true;
    let _ = replace_completed_vehicle(world, old, &input);
}

#[test]
fn fatal_lifecycle_error_stops_catch_up_and_preserves_the_full_backlog() {
    let ReplaceFixture {
        session,
        old,
        profile,
    } = fixture(VehicleStatus::Active, false, 0);
    let replacement = input(&session, profile, 10.0);
    let mut app = App::new();
    app.add_plugins((TimePlugin, LaneFlowPlugin));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    app.insert_resource(session);
    app.insert_resource(FatalPolicy {
        old,
        input: replacement,
        attempted: false,
    });
    app.add_systems(
        LaneFlowFixed,
        fail_once_at_boundary.in_set(LaneFlowFixedSet::Lifecycle),
    );
    app.update();

    *app.world_mut().resource_mut::<TimeUpdateStrategy>() =
        TimeUpdateStrategy::ManualDuration(Duration::from_millis(40));
    app.update();

    let session = app.world().resource::<LaneFlowSession>();
    assert_eq!(session.core().tick_index(), 0);
    assert_eq!(session.frame_report().steps_run(), 0);
    assert_eq!(session.frame_report().backlog(), Duration::from_millis(40));
    assert!(matches!(
        session.last_error(),
        Some(LaneFlowAdapterError::CoreVehicleReplace { .. })
    ));

    *app.world_mut().resource_mut::<TimeUpdateStrategy>() =
        TimeUpdateStrategy::ManualDuration(Duration::ZERO);
    app.update();
    let session = app.world().resource::<LaneFlowSession>();
    assert_eq!(session.core().tick_index(), 2);
    assert_eq!(session.frame_report().steps_run(), 2);
    assert_eq!(session.frame_report().backlog(), Duration::ZERO);
    assert!(session.last_error().is_none());
}

#[test]
fn missing_session_is_a_structured_error() {
    let ReplaceFixture {
        session,
        old,
        profile,
    } = fixture(VehicleStatus::Completed, false, 0);
    let replacement = input(&session, profile, 10.0);
    let mut world = World::new();

    assert!(matches!(
        replace_completed_vehicle(&mut world, old, &replacement),
        Err(LaneFlowAdapterError::MissingSessionForLifecycleCommand)
    ));
}
