#![allow(
    dead_code,
    reason = "shared allocation, performance, and Criterion fixture"
)]

use std::{num::NonZeroU32, time::Duration};

use bevy_app::{App, PostUpdate};
use bevy_ecs::{entity::Entity, hierarchy::ChildOf};
use bevy_time::{TimePlugin, TimeUpdateStrategy};
use bevy_transform::{
    TransformPlugin,
    components::{GlobalTransform, Transform},
};
use laneflow_bevy::{
    LaneFlowFramePlacement, LaneFlowPlugin, LaneFlowPresentationReport, LaneFlowSession,
    LaneFlowSessionConfig,
};
use laneflow_core::{
    CoreWorld, EdgeLength, EdgeProgress, IidmProfileSpec, InitialTrafficData, LaneEdge, LaneGraph,
    ParkingRegistry, Route, SignalRegistry, VehicleHandle, VehicleProfile, VehicleProfileRegistry,
    VehicleSpawnInput,
};
use laneflow_spatial::{
    CanonicalFrameId, CanonicalPoint3F32, FramePlacementToken, SpatialEdgeInput, SpatialRegistry,
};

pub const TEN_THOUSAND: usize = 10_000;
pub const ONE_HUNDRED_THOUSAND: usize = 100_000;

const VEHICLE_SPACING_METERS: f64 = 0.25;

pub struct PresentationScaleFixture {
    app: App,
    proxies: Vec<Entity>,
}

impl PresentationScaleFixture {
    pub fn new(count: usize) -> Self {
        assert!(count > 0, "scale fixture requires at least one vehicle");
        let edge_length = VEHICLE_SPACING_METERS * count as f64;
        let graph = LaneGraph::try_new([LaneEdge::new(
            "scale-edge",
            EdgeLength::try_new(edge_length).expect("valid scale edge length"),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            std::iter::empty::<&str>(),
        )])
        .expect("valid scale graph");
        let edge = graph.edge_handle("scale-edge").expect("scale edge");
        let profiles = VehicleProfileRegistry::try_new([scale_profile()])
            .expect("valid scale profile registry");
        let profile = profiles
            .profile_handle("scale-profile")
            .expect("scale profile");
        let traffic = InitialTrafficData::try_new_with_signals_and_parking(
            graph.clone(),
            [Route::try_new("scale-route", ["scale-edge"]).expect("valid scale route")],
            profiles,
            SignalRegistry::empty(),
            ParkingRegistry::empty(),
        )
        .expect("valid scale traffic data");
        let vehicles = (0..count)
            .map(|index| {
                VehicleSpawnInput::stopped(
                    format!("vehicle-{index:06}"),
                    profile,
                    "scale-route",
                    0,
                    EdgeProgress::try_new(VEHICLE_SPACING_METERS * (index as f64 + 0.5))
                        .expect("valid scale progress"),
                )
            })
            .collect();
        let core =
            CoreWorld::with_traffic_data(16, traffic, vehicles).expect("valid scale Core world");
        let vehicle_handles: Vec<VehicleHandle> =
            core.vehicles().map(|vehicle| vehicle.handle).collect();

        let half_length = edge_length as f32 / 2.0;
        let points = [
            CanonicalPoint3F32::try_new(-half_length, 0.0, 0.0).expect("valid scale start point"),
            CanonicalPoint3F32::try_new(half_length, 0.0, 0.0).expect("valid scale end point"),
        ];
        let spatial = SpatialRegistry::try_new(
            &graph,
            CanonicalFrameId::try_new("validation/bevy-scale").expect("valid scale frame"),
            [SpatialEdgeInput::new(edge, &points)],
        )
        .expect("valid scale Spatial registry");
        let config = LaneFlowSessionConfig::new(NonZeroU32::new(1).expect("non-zero"));
        let mut session = LaneFlowSession::with_pose_capacity(core, spatial, config, count);

        let mut app = App::new();
        app.add_plugins((TimePlugin, TransformPlugin, LaneFlowPlugin));
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
        let root = app.world_mut().spawn(Transform::IDENTITY).id();
        let mut proxies = Vec::with_capacity(count);
        for vehicle in vehicle_handles {
            let proxy = app
                .world_mut()
                .spawn((Transform::IDENTITY, ChildOf(root)))
                .id();
            session
                .bind_vehicle_entity(vehicle, proxy)
                .expect("scale vehicle binding");
            proxies.push(proxy);
        }
        session
            .set_frame_placement(LaneFlowFramePlacement::new(
                root,
                FramePlacementToken::new(1),
            ))
            .expect("scale frame placement");
        app.insert_resource(session);
        app.update();

        let fixture = Self { app, proxies };
        fixture.assert_presented(count);
        fixture
    }

    pub fn run_post_update(&mut self) {
        self.app.world_mut().run_schedule(PostUpdate);
    }

    pub fn presentation_report(&self) -> LaneFlowPresentationReport {
        self.app
            .world()
            .resource::<LaneFlowSession>()
            .presentation_report()
    }

    pub fn assert_presented(&self, count: usize) {
        let session = self.app.world().resource::<LaneFlowSession>();
        assert!(session.last_error().is_none(), "scale presentation error");
        let report = session.presentation_report();
        assert_eq!(report.pose_records(), count);
        assert_eq!(report.mapped_records(), count);
        assert_eq!(report.unbound_records(), 0);
        assert_eq!(report.applied_records(), count);
        assert!(session.pose_input_capacity() >= count);
        assert!(session.pose_batch_capacity() >= count);
        assert!(session.pose_scratch_capacity() >= count);
        assert!(session.transform_staging_capacity() >= count);
        assert!(session.vehicle_entities().capacity() >= count);

        for entity in [self.proxies[0], self.proxies[count - 1]] {
            assert!(
                self.app
                    .world()
                    .get::<GlobalTransform>(entity)
                    .expect("scale GlobalTransform")
                    .translation()
                    .is_finite()
            );
        }
    }
}

fn scale_profile() -> VehicleProfile {
    VehicleProfile::try_new_iidm(
        "scale-profile",
        IidmProfileSpec {
            length: 0.1,
            desired_speed: 1.0,
            min_gap: 0.0,
            time_headway: 1.0,
            max_acceleration: 1.0,
            comfortable_deceleration: 1.0,
            emergency_deceleration: 2.0,
        },
    )
    .expect("valid scale profile")
}
