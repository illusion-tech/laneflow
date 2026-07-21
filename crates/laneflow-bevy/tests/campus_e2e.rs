use std::{num::NonZeroU32, time::Duration};

use bevy_app::App;
use bevy_ecs::{entity::Entity, hierarchy::ChildOf};
use bevy_time::{TimePlugin, TimeUpdateStrategy};
use bevy_transform::{
    TransformPlugin,
    components::{GlobalTransform, Transform},
};
use laneflow_bevy::{
    LaneFlowFramePlacement, LaneFlowPlugin, LaneFlowSession, LaneFlowSessionConfig,
};
use laneflow_core::{CoreWorld, EdgeProgress, Speed, VehicleHandle, VehicleSpawnInput};
use laneflow_data::{NamedArtifact, from_scenario_json_slice};
use laneflow_spatial::{FramePlacementToken, SpatialEdgeInput, SpatialRegistry};

const MANIFEST: &[u8] = include_bytes!("../../../examples/data/v0.1-campus.scenario.json");
const TRAFFIC: &[u8] =
    include_bytes!("../../../examples/data/v0.5-empty-signals-and-parking.laneflow.json");
const SPATIAL: &[u8] = include_bytes!("../../../examples/data/v0.1-campus.spatial.json");

struct CampusFixture {
    app: App,
    vehicles: Vec<VehicleHandle>,
    proxies: Vec<Entity>,
}

impl CampusFixture {
    fn new() -> Self {
        let loaded = from_scenario_json_slice(
            MANIFEST,
            &[
                NamedArtifact::new("v0.5-empty-signals-and-parking.laneflow.json", TRAFFIC),
                NamedArtifact::new("v0.1-campus.spatial.json", SPATIAL),
            ],
        )
        .expect("campus scenario and artifact digests load");
        let (traffic, loaded_spatial) = loaded.into_parts();
        let traffic = traffic.into_initial_traffic_data();
        let graph = traffic.lane_graph().clone();
        let spatial = SpatialRegistry::try_new(
            &graph,
            loaded_spatial.frame_id().clone(),
            loaded_spatial
                .edges()
                .iter()
                .map(|edge| SpatialEdgeInput::new(edge.edge(), edge.points())),
        )
        .expect("campus Spatial registry");
        let profile = traffic
            .vehicle_profiles()
            .profile_handle("passenger-car")
            .expect("campus passenger profile");
        let core = CoreWorld::with_traffic_data(
            16,
            traffic,
            vec![
                VehicleSpawnInput::active(
                    "campus-main",
                    profile,
                    "main-route",
                    0,
                    EdgeProgress::try_new(6.0).expect("main progress"),
                    Speed::try_new(2.0).expect("main speed"),
                ),
                VehicleSpawnInput::active(
                    "campus-loop",
                    profile,
                    "loop-once",
                    0,
                    EdgeProgress::try_new(4.0).expect("loop progress"),
                    Speed::try_new(1.0).expect("loop speed"),
                ),
            ],
        )
        .expect("campus Core world");
        let vehicles: Vec<_> = core.vehicles().map(|vehicle| vehicle.handle).collect();
        let config = LaneFlowSessionConfig::new(NonZeroU32::new(4).expect("non-zero"));
        let mut session =
            LaneFlowSession::with_pose_capacity(core, spatial, config, vehicles.len());

        let mut app = App::new();
        app.add_plugins((TimePlugin, TransformPlugin, LaneFlowPlugin));
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
        let root = app
            .world_mut()
            .spawn(Transform::from_xyz(100.0, 3.0, -7.0))
            .id();
        let mut proxies = Vec::with_capacity(vehicles.len());
        for vehicle in &vehicles {
            let proxy = app
                .world_mut()
                .spawn((Transform::IDENTITY, ChildOf(root)))
                .id();
            session
                .bind_vehicle_entity(*vehicle, proxy)
                .expect("campus vehicle binding");
            proxies.push(proxy);
        }
        session
            .set_frame_placement(LaneFlowFramePlacement::new(
                root,
                FramePlacementToken::new(1),
            ))
            .expect("campus frame placement");
        app.insert_resource(session);
        app.update();

        Self {
            app,
            vehicles,
            proxies,
        }
    }

    fn update_with_delta(&mut self, milliseconds: u64) {
        *self.app.world_mut().resource_mut::<TimeUpdateStrategy>() =
            TimeUpdateStrategy::ManualDuration(Duration::from_millis(milliseconds));
        self.app.update();
    }

    fn transforms_in_core_order(&self) -> Vec<([f32; 3], [f32; 4], [f32; 3])> {
        let world = self.app.world();
        let session = world.resource::<LaneFlowSession>();
        let ordered_entities: Vec<_> = session
            .core()
            .vehicles()
            .map(|vehicle| {
                session
                    .vehicle_entities()
                    .entity(vehicle.handle)
                    .expect("every campus vehicle remains mapped")
            })
            .collect();
        assert_eq!(ordered_entities, self.proxies, "stable presentation order");
        ordered_entities
            .into_iter()
            .map(|entity| {
                let local = world.get::<Transform>(entity).expect("local Transform");
                let global = world
                    .get::<GlobalTransform>(entity)
                    .expect("global Transform");
                (
                    local.translation.to_array(),
                    local.rotation.to_array(),
                    global.translation().to_array(),
                )
            })
            .collect()
    }
}

#[test]
fn campus_load_fixed_step_spatial_batch_and_bevy_apply_are_partition_deterministic() {
    let mut partitioned = CampusFixture::new();
    let mut batched = CampusFixture::new();

    for delta in [7, 9, 16] {
        partitioned.update_with_delta(delta);
    }
    batched.update_with_delta(32);

    let partitioned_session = partitioned.app.world().resource::<LaneFlowSession>();
    let batched_session = batched.app.world().resource::<LaneFlowSession>();
    assert_eq!(partitioned_session.core(), batched_session.core());
    assert_eq!(partitioned_session.core().tick_index(), 2);
    assert_eq!(partitioned_session.core().time_ms(), 32);
    assert_eq!(partitioned_session.accumulator(), Duration::ZERO);
    assert_eq!(
        partitioned_session.accumulator(),
        batched_session.accumulator()
    );
    assert!(partitioned_session.last_error().is_none());
    assert!(batched_session.last_error().is_none());
    for session in [partitioned_session, batched_session] {
        let report = session.presentation_report();
        assert_eq!(report.pose_records(), 2);
        assert_eq!(report.mapped_records(), 2);
        assert_eq!(report.unbound_records(), 0);
        assert_eq!(report.applied_records(), 2);
    }
    assert_eq!(partitioned.vehicles, batched.vehicles);
    assert_eq!(
        partitioned.transforms_in_core_order(),
        batched.transforms_in_core_order()
    );
    assert!(
        partitioned
            .transforms_in_core_order()
            .iter()
            .all(|(local, rotation, global)| local
                .iter()
                .chain(rotation)
                .chain(global)
                .all(|component| component.is_finite()))
    );
}
