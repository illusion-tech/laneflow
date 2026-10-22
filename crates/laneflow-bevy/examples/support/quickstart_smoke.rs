#[path = "quickstart_presentation.rs"]
mod presentation;
#[path = "quickstart_scene.rs"]
mod quickstart_scene;

use std::{num::NonZeroU32, time::Duration};

use bevy_app::App;
use bevy_math::Vec3;
use bevy_time::{TimePlugin, TimeUpdateStrategy};
use bevy_transform::{TransformPlugin, components::Transform};
use laneflow_bevy::{
    LaneFlowAdapterError, LaneFlowCommittedPoseBatch, LaneFlowSession, LaneFlowSessionConfig,
};
use laneflow_spatial::FramePlacementToken;
use presentation::{Presentation, PresentationError};

fn app(session: LaneFlowSession) -> App {
    let mut app = App::new();
    app.add_plugins((TimePlugin, TransformPlugin));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        100,
    )));
    presentation::install(&mut app, session).expect("shared presentation setup");
    app
}

#[test]
fn every_vehicle_is_bound_and_receives_its_complete_committed_pose() {
    let mut app = app(quickstart_scene::session().expect("shared scene setup"));
    let vehicles: Vec<_> = app
        .world()
        .resource::<LaneFlowSession>()
        .world()
        .committed_pose_sources()
        .map(|(vehicle, _)| vehicle)
        .collect();
    assert_eq!(vehicles.len(), 2, "fixture vehicle count");
    let entities: Vec<_> = vehicles
        .iter()
        .map(|vehicle| {
            app.world()
                .resource::<LaneFlowSession>()
                .vehicle_entity(*vehicle)
                .expect("all vehicles bound")
        })
        .collect();
    assert_ne!(entities[0], entities[1], "distinct vehicle proxies");
    app.update();
    let before: Vec<_> = entities
        .iter()
        .map(|entity| *app.world().get::<Transform>(*entity).unwrap())
        .collect();
    for _ in 0..16 {
        app.update();
    }
    assert!(app.world().resource::<Presentation>().last_error.is_none());
    let mut poses = LaneFlowCommittedPoseBatch::new();
    let mut session = app.world_mut().resource_mut::<LaneFlowSession>();
    assert!(session.frame_report().steps_run() > 0);
    session
        .extract_committed_pose_batch(FramePlacementToken::new(1), &mut poses)
        .expect("committed oracle");
    assert_eq!(poses.vehicles(), vehicles);
    for ((entity, before), record) in entities.iter().zip(before).zip(poses.batch().records()) {
        let after = app.world().get::<Transform>(*entity).unwrap();
        let pose = record.pose();
        let position = pose.position();
        let tangent = pose.tangent();
        let up = pose.up();
        assert_ne!(after.translation, before.translation, "every vehicle moves");
        assert_eq!(
            after.translation,
            Vec3::new(position.x(), position.y(), position.z())
        );
        assert!(after.translation.is_finite() && after.rotation.is_finite());
        assert!(
            after
                .forward()
                .dot(Vec3::new(tangent.x(), tangent.y(), tangent.z()))
                > 0.999_99,
            "forward follows tangent"
        );
        assert!(
            after.up().dot(Vec3::new(up.x(), up.y(), up.z())) > 0.999_99,
            "up follows canonical up"
        );
    }
}

#[test]
fn a_missing_second_transform_rejects_the_whole_presentation_batch() {
    let mut app = app(quickstart_scene::session().unwrap());
    app.update();
    let session = app.world().resource::<LaneFlowSession>();
    let vehicles: Vec<_> = session
        .world()
        .committed_pose_sources()
        .map(|(vehicle, _)| vehicle)
        .collect();
    let first = session.vehicle_entity(vehicles[0]).unwrap();
    let second = session.vehicle_entity(vehicles[1]).unwrap();
    let before = *app.world().get::<Transform>(first).unwrap();
    app.world_mut().entity_mut(second).remove::<Transform>();
    app.update();
    assert_eq!(
        *app.world().get::<Transform>(first).unwrap(),
        before,
        "no partial write"
    );
    assert_eq!(
        app.world().resource::<Presentation>().last_error,
        Some(PresentationError::MissingTransform {
            vehicle: vehicles[1],
            entity: second
        })
    );
}

#[test]
fn extraction_failure_is_recorded_and_does_not_update_proxies() {
    let session = LaneFlowSession::new(
        quickstart_scene::world().unwrap(),
        None,
        LaneFlowSessionConfig::new(NonZeroU32::MIN),
    )
    .unwrap();
    let mut app = app(session);
    app.update();
    assert_eq!(
        app.world().resource::<Presentation>().last_error,
        Some(PresentationError::Extraction(
            LaneFlowAdapterError::PoseExtractionWithoutSpatial
        ))
    );
    let session = app.world().resource::<LaneFlowSession>();
    for (vehicle, _) in session.world().committed_pose_sources() {
        let entity = session.vehicle_entity(vehicle).unwrap();
        assert_eq!(
            *app.world().get::<Transform>(entity).unwrap(),
            Transform::IDENTITY
        );
    }
}
