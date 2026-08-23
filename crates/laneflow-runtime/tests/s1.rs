use std::sync::Arc;

use laneflow_format::{FormatLimits, check_canonical_network_input_v1};
use laneflow_runtime::{PoseSource, TickInput, TrafficWorld, VehicleSpawnInput, WorldConfig};
use laneflow_spatial::{
    CanonicalPoseBatch, FramePlacementToken, PoseInput, PoseRecordId, SpatialSession,
};
use laneflow_static_contract::{StaticRouteOrdinal, VehicleProfileOrdinal};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};

const FULL_SPATIAL: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable-v1/lfca-v1-full-spatial/expected.lfca"
);

fn revision() -> Arc<laneflow_static_network::SharedNetworkRevision> {
    let input = check_canonical_network_input_v1(FULL_SPATIAL, FormatLimits::V1_HARD)
        .expect("checked canonical network input");
    build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .expect("shared network revision")
}

#[test]
fn s1_two_vehicles_step_and_extract_pose_batch() {
    let revision = revision();
    let world = TrafficWorld::install(Arc::clone(&revision), WorldConfig::new(8, 4, 1, 100))
        .expect("install");
    assert!(Arc::ptr_eq(&world.revision(), &revision));

    let mut world = world;
    let route = world
        .static_route(StaticRouteOrdinal::from_raw(0))
        .expect("static route");
    let profile = world
        .traffic()
        .relations()
        .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
        .expect("profile");
    let leader = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            1.0 + profile.length() + profile.min_gap() + 2.0,
            0.0,
        ))
        .expect("leader");
    let follower = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            1.0,
            0.0,
        ))
        .expect("follower");

    for _ in 0..12 {
        world.step(TickInput::new(100)).expect("step");
    }

    let poses = world.committed_pose_sources();
    assert_eq!(poses.as_slice().len(), 2);
    let leader_source = poses
        .as_slice()
        .iter()
        .find(|(handle, _)| *handle == leader)
        .map(|(_, source)| *source)
        .expect("leader pose");
    let follower_source = poses
        .as_slice()
        .iter()
        .find(|(handle, _)| *handle == follower)
        .map(|(_, source)| *source)
        .expect("follower pose");
    let PoseSource::Lane { .. } = leader_source else {
        panic!("leader must stay on a lane");
    };
    let PoseSource::Lane { .. } = follower_source else {
        panic!("follower must stay on a lane");
    };
    let _ = profile;

    let mut session = SpatialSession::bind(world.revision())
        .expect("bind")
        .expect("full spatial session");
    assert!(Arc::ptr_eq(&session.revision(), &world.revision()));
    let mut batch = CanonicalPoseBatch::new();
    let inputs: Vec<PoseInput> = poses
        .as_slice()
        .iter()
        .enumerate()
        .map(|(index, (_, source))| match *source {
            PoseSource::Lane { edge, progress } => PoseInput::from_source(
                PoseRecordId::new(index as u32),
                laneflow_spatial::PoseSource::Lane { edge, progress },
            ),
            PoseSource::Parking { space } => PoseInput::from_source(
                PoseRecordId::new(index as u32),
                laneflow_spatial::PoseSource::Parking { space },
            ),
        })
        .collect();
    session
        .extract_pose_batch(FramePlacementToken::new(1), &inputs, &mut batch)
        .expect("extract");
    assert_eq!(batch.records().len(), 2);
    let _ = (leader, follower);
}
