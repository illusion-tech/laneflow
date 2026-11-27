#[path = "support/policy.rs"]
mod test_policy;

use std::sync::Arc;

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{
    ParkedVehicleSpawnInput, ParkingTarget, PoseSource, RouteHandle, RouteRegisterInput, TickInput,
    TrafficWorld, VehicleSpawnInput, WorldConfig, WorldConfigInput, WorldId, WorldInstallInput,
};
use laneflow_spatial::{
    CanonicalPoseBatch, FramePlacementToken, PoseInput, PoseRecordId, SpatialSession,
};
use laneflow_static_contract::{LaneEdgeOrdinal, ParkingSpaceOrdinal, VehicleProfileOrdinal};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};

fn install_fixture(
    revision: std::sync::Arc<laneflow_static_network::SharedNetworkRevision>,
    config: laneflow_runtime::WorldConfig,
) -> Result<laneflow_runtime::TrafficWorld, laneflow_runtime::InstallError> {
    let origin = *revision.canonical_origin();
    laneflow_runtime::TrafficWorld::install(WorldInstallInput {
        revision: std::sync::Arc::clone(&revision),
        config,
        execution: laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        source: laneflow_runtime::CommittedNetworkSource::Published {
            reference: laneflow_runtime::PublishedLfcaReference::new(
                "fixture://in-process",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("non-empty fixture key"),
        },
        world_id: WorldId::new(0),
        policy_selection: test_policy::selection(&revision),
    })
}

const FULL_SPATIAL: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfca"
);

fn edge_for_length(world: &TrafficWorld, length: u32) -> LaneEdgeOrdinal {
    let index = world
        .traffic()
        .lane_lengths_millimetres()
        .iter()
        .position(|actual| *actual == length)
        .expect("fixture lane length");
    LaneEdgeOrdinal::try_from_usize(index).expect("fixture lane ordinal")
}

fn revision() -> Arc<laneflow_static_network::SharedNetworkRevision> {
    let input = check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD)
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

fn fixture_world_and_route() -> (TrafficWorld, RouteHandle) {
    let revision = revision();
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::try_new(WorldConfigInput {
            vehicle_capacity: 8,
            route_capacity: 4,
            route_edge_occurrence_capacity: 1_024,
            route_conflict_occurrence_capacity: 1_024,
            fixed_delta_time_ms: 100,
        })
        .expect("有效的交通配置"),
    )
    .expect("install");
    assert!(Arc::ptr_eq(&world.revision(), &revision));

    let first = edge_for_length(&world, 10_000);
    let middle = edge_for_length(&world, 8_000);
    let last = edge_for_length(&world, 12_000);
    let route = world
        .register_route(RouteRegisterInput::new(vec![first, middle, last]))
        .expect("register");
    (world, route)
}

#[test]
fn s1_two_vehicles_step_and_extract_pose_batch() {
    let (mut world, route) = fixture_world_and_route();
    let profile = world
        .traffic()
        .relations()
        .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
        .expect("profile");
    let leader = world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000 + profile.length_mm() + profile.min_gap_mm() + 2_000,
                0,
            )
            .with_open_entrance(),
        )
        .expect("leader");
    let follower = world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 1_000, 0)
                .with_open_entrance(),
        )
        .expect("follower");

    for _ in 0..12 {
        world.step(TickInput::new(100)).expect("step");
    }

    let poses = world.committed_pose_sources().collect::<Vec<_>>();
    let [
        (first, PoseSource::Lane { .. }),
        (second, PoseSource::Lane { .. }),
    ] = poses.as_slice()
    else {
        panic!("expected two lane sources");
    };
    assert_eq!((*first, *second), (leader, follower));

    let mut session = SpatialSession::bind(world.revision())
        .expect("bind")
        .expect("full spatial session");
    assert!(Arc::ptr_eq(&session.revision(), &world.revision()));
    let mut batch = CanonicalPoseBatch::new();
    let inputs: Vec<_> = (0..)
        .zip(poses)
        .map(|(index, (_, source))| PoseInput::from_source(PoseRecordId::new(index), source))
        .collect();
    session
        .extract_pose_batch(FramePlacementToken::new(1), &inputs, &mut batch)
        .expect("extract");
    let [front, rear] = batch.records() else {
        panic!("expected two pose records");
    };
    assert_eq!((front.record().raw(), rear.record().raw()), (0, 1));
    assert!(front.pose().position().x() > rear.pose().position().x());
}

#[test]
fn selected_lane_and_parking_sources_sample_in_caller_order() {
    let (mut world, route) = fixture_world_and_route();
    let profile = VehicleProfileOrdinal::from_raw(0);
    let active = world
        .spawn_vehicle(VehicleSpawnInput::new(profile, route, 0, 1_000, 0).with_open_entrance())
        .expect("spawn on entry lane");
    let parked = world
        .spawn_parked_vehicle(
            ParkedVehicleSpawnInput::new(profile, route, 0, 0),
            ParkingTarget::ExplicitSpace(ParkingSpaceOrdinal::from_raw(0)),
        )
        .expect("spawn in explicit parking space")
        .vehicle;

    let inputs = [(71, parked), (19, active)].map(|(record, vehicle)| {
        let source = world
            .committed_pose_source(vehicle)
            .expect("live vehicle")
            .expect("committed source");
        PoseInput::from_source(PoseRecordId::new(record), source)
    });
    let mut session = SpatialSession::bind(world.revision())
        .expect("bind")
        .expect("full spatial session");
    let mut batch = CanonicalPoseBatch::new();
    session
        .extract_pose_batch(FramePlacementToken::new(1), &inputs, &mut batch)
        .expect("extract selected sources");

    let [parking, lane] = batch.records() else {
        panic!("expected parking and lane records");
    };
    assert_eq!((parking.record().raw(), lane.record().raw()), (71, 19));
    // 直道沿 +X；泊位入口在 4 m，向右偏移 3 m，独立于停放车辆保留的 0 mm 游标。
    let position = parking.pose().position();
    assert_eq!((position.x(), position.y(), position.z()), (4.0, 0.0, 3.0));
    let position = lane.pose().position();
    assert_eq!((position.x(), position.y(), position.z()), (1.0, 0.0, 0.0));

    let tangent = lane.pose().tangent();
    assert_eq!((tangent.x(), tangent.y(), tangent.z()), (1.0, 0.0, 0.0));
    let up = lane.pose().up();
    assert_eq!((up.x(), up.y(), up.z()), (0.0, 1.0, 0.0));
    assert_eq!(parking.pose().up(), up);

    // 泊位从 +X 朝 −Z 旋转 0.25 rad；按空间几何设计中的 0.5° 切线角误差验收。
    let tangent = parking.pose().tangent();
    assert_eq!(tangent.y(), 0.0);
    let heading = f64::from(-tangent.z()).atan2(f64::from(tangent.x()));
    assert!((heading - 0.25).abs() <= 0.5_f64.to_radians(), "{heading}");
}
