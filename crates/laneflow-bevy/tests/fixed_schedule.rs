use std::{num::NonZeroU32, time::Duration};

use bevy_app::App;
use bevy_time::{Fixed, Time, TimePlugin, TimeUpdateStrategy};
use laneflow_bevy::{LaneFlowAdapterError, LaneFlowPlugin, LaneFlowSession, LaneFlowSessionConfig};
use laneflow_core::{CoreWorld, LaneGraph};
use laneflow_spatial::{CanonicalFrameId, SpatialRegistry};

fn empty_session(fixed_delta_time_ms: u64, max_catch_up_steps: u32) -> LaneFlowSession {
    let lane_graph = LaneGraph::empty();
    let spatial = SpatialRegistry::try_new(
        &lane_graph,
        CanonicalFrameId::try_new("test:frame").expect("valid frame"),
        [],
    )
    .expect("empty graph has an empty spatial registry");
    let core = CoreWorld::new(fixed_delta_time_ms).expect("valid fixed delta");
    let config = LaneFlowSessionConfig::new(
        NonZeroU32::new(max_catch_up_steps).expect("test catch-up limit is non-zero"),
    );
    LaneFlowSession::new(core, spatial, config)
}

fn headless_app(session: LaneFlowSession) -> App {
    let mut app = App::new();
    app.add_plugins((TimePlugin, LaneFlowPlugin));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    app.insert_resource(session);
    app.update();
    app
}

fn update_with_delta(app: &mut App, delta: Duration) {
    *app.world_mut().resource_mut::<TimeUpdateStrategy>() =
        TimeUpdateStrategy::ManualDuration(delta);
    app.update();
}

#[test]
fn outer_frame_runs_zero_one_and_multiple_fixed_steps() {
    let mut app = headless_app(empty_session(10, 4));

    update_with_delta(&mut app, Duration::from_millis(9));
    let session = app.world().resource::<LaneFlowSession>();
    assert_eq!(session.core().tick_index(), 0);
    assert_eq!(session.frame_report().steps_run(), 0);
    assert_eq!(session.frame_report().backlog(), Duration::from_millis(9));

    update_with_delta(&mut app, Duration::from_millis(1));
    let session = app.world().resource::<LaneFlowSession>();
    assert_eq!(session.core().tick_index(), 1);
    assert_eq!(session.frame_report().steps_run(), 1);
    assert_eq!(session.frame_step_results().len(), 1);
    assert_eq!(session.frame_report().backlog(), Duration::ZERO);

    update_with_delta(&mut app, Duration::from_millis(30));
    let session = app.world().resource::<LaneFlowSession>();
    assert_eq!(session.core().tick_index(), 4);
    assert_eq!(session.core().time_ms(), 40);
    assert_eq!(session.frame_report().steps_run(), 3);
    assert_eq!(session.frame_step_results().len(), 3);
    assert_eq!(session.frame_report().backlog(), Duration::ZERO);
    assert!(!session.frame_report().catch_up_limit_reached());
}

#[test]
fn catch_up_limit_preserves_backlog_for_later_frames() {
    let mut app = headless_app(empty_session(10, 2));

    update_with_delta(&mut app, Duration::from_millis(35));
    let session = app.world().resource::<LaneFlowSession>();
    assert_eq!(session.core().tick_index(), 2);
    assert_eq!(session.frame_report().steps_run(), 2);
    assert_eq!(session.frame_report().backlog(), Duration::from_millis(15));
    assert!(session.frame_report().catch_up_limit_reached());

    update_with_delta(&mut app, Duration::ZERO);
    let session = app.world().resource::<LaneFlowSession>();
    assert_eq!(session.core().tick_index(), 3);
    assert_eq!(session.frame_report().steps_run(), 1);
    assert_eq!(session.frame_report().backlog(), Duration::from_millis(5));
    assert!(!session.frame_report().catch_up_limit_reached());
}

#[test]
fn equal_elapsed_time_is_independent_of_outer_frame_partitioning() {
    let mut partitioned = headless_app(empty_session(10, 4));
    let mut batched = headless_app(empty_session(10, 4));

    for delta in [7, 8, 15] {
        update_with_delta(&mut partitioned, Duration::from_millis(delta));
    }
    update_with_delta(&mut batched, Duration::from_millis(30));

    let partitioned = partitioned.world().resource::<LaneFlowSession>();
    let batched = batched.world().resource::<LaneFlowSession>();
    assert_eq!(partitioned.core(), batched.core());
    assert_eq!(partitioned.accumulator(), batched.accumulator());
    assert_eq!(partitioned.core().tick_index(), 3);
    assert_eq!(partitioned.core().time_ms(), 30);
}

#[test]
fn sub_millisecond_frame_time_is_not_dropped() {
    let mut app = headless_app(empty_session(1, 2));

    update_with_delta(&mut app, Duration::from_micros(500));
    let session = app.world().resource::<LaneFlowSession>();
    assert_eq!(session.core().tick_index(), 0);
    assert_eq!(session.accumulator(), Duration::from_micros(500));

    update_with_delta(&mut app, Duration::from_micros(500));
    let session = app.world().resource::<LaneFlowSession>();
    assert_eq!(session.core().tick_index(), 1);
    assert_eq!(session.accumulator(), Duration::ZERO);
}

#[test]
fn missing_time_resource_is_a_structured_error() {
    let mut app = App::new();
    app.add_plugins(LaneFlowPlugin);
    app.insert_resource(empty_session(10, 1));

    app.update();

    let session = app.world().resource::<LaneFlowSession>();
    assert!(matches!(
        session.last_error(),
        Some(LaneFlowAdapterError::MissingTimeResource)
    ));
    assert_eq!(session.core().tick_index(), 0);
    assert_eq!(session.accumulator(), Duration::ZERO);
}

#[test]
fn lane_flow_driver_does_not_change_bevy_fixed_clock() {
    let mut control = App::new();
    control.add_plugins(TimePlugin);
    control.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    control.update();

    let mut integrated = headless_app(empty_session(10, 4));

    for delta in [Duration::from_millis(17), Duration::from_millis(31)] {
        *control.world_mut().resource_mut::<TimeUpdateStrategy>() =
            TimeUpdateStrategy::ManualDuration(delta);
        control.update();
        update_with_delta(&mut integrated, delta);
    }

    let control_time = control.world().resource::<Time<Fixed>>();
    let integrated_time = integrated.world().resource::<Time<Fixed>>();
    assert_eq!(integrated_time.timestep(), control_time.timestep());
    assert_eq!(integrated_time.elapsed(), control_time.elapsed());
    assert_eq!(integrated_time.delta(), control_time.delta());
    assert_eq!(integrated_time.overstep(), control_time.overstep());
}

#[test]
fn session_preserves_requested_pose_scratch_capacity() {
    let lane_graph = LaneGraph::empty();
    let spatial = SpatialRegistry::try_new(
        &lane_graph,
        CanonicalFrameId::try_new("test:capacity").expect("valid frame"),
        [],
    )
    .expect("empty registry");
    let core = CoreWorld::new(10).expect("valid world");
    let config = LaneFlowSessionConfig::new(NonZeroU32::new(1).expect("non-zero"));

    let session = LaneFlowSession::with_pose_capacity(core, spatial, config, 10_000);

    assert!(session.pose_scratch_capacity() >= 10_000);
}
