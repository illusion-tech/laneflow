use std::f64::consts::FRAC_PI_2;

use laneflow_core::{
    CoreWorld, EdgeLength, EdgeProgress, IidmProfileSpec, InitialTrafficData, LaneEdge, LaneGraph,
    ParkingRegistry, ParkingSpace, ParkingSpaceGeometry, Route, SignalRegistry, Speed,
    VehicleHandle, VehicleProfile, VehicleProfileRegistry, VehicleSpawnInput,
};
use laneflow_spatial::{
    CanonicalFrameId, CanonicalPoint3F32, CanonicalPoseBatchF32, CanonicalPoseBatchScratch,
    FramePlacementToken, PoseInputRecord, SpatialAxis, SpatialEdgeInput, SpatialError,
    SpatialRegistry,
};

struct Fixture {
    spatial: SpatialRegistry,
    parking: ParkingRegistry,
    edge: laneflow_core::EdgeHandle,
    space: laneflow_core::ParkingSpaceHandle,
    vehicles: [VehicleHandle; 2],
}

fn point(x: f32, y: f32, z: f32) -> CanonicalPoint3F32 {
    CanonicalPoint3F32::try_new(x, y, z).expect("valid canonical point")
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

fn fixture() -> Fixture {
    let graph = LaneGraph::try_new([LaneEdge::new(
        "edge",
        EdgeLength::try_new(100.0).expect("valid edge length"),
        std::iter::empty::<&str>(),
    )])
    .expect("valid graph");
    let edge = graph.edge_handle("edge").expect("edge handle");
    let parking = ParkingRegistry::try_new(
        &graph,
        std::iter::empty(),
        [ParkingSpace::new(
            "space",
            None,
            "edge",
            25.0,
            "edge",
            25.0,
            ParkingSpaceGeometry::new(2.0, FRAC_PI_2, 5.0, 2.4),
        )],
    )
    .expect("valid parking registry");
    let space = parking.space_handle("space").expect("space handle");

    let profiles = VehicleProfileRegistry::try_new([profile()]).expect("valid profiles");
    let profile = profiles.profile_handle("profile").expect("profile handle");
    let traffic = InitialTrafficData::try_new_with_signals_and_parking(
        graph.clone(),
        [Route::try_new("route", ["edge"]).expect("valid route")],
        profiles,
        SignalRegistry::empty(),
        parking.clone(),
    )
    .expect("valid traffic data");
    let world = CoreWorld::with_traffic_data(
        16,
        traffic,
        vec![
            VehicleSpawnInput::active(
                "vehicle-a",
                profile,
                "route",
                0,
                EdgeProgress::try_new(10.0).expect("valid progress"),
                Speed::ZERO,
            ),
            VehicleSpawnInput::active(
                "vehicle-b",
                profile,
                "route",
                0,
                EdgeProgress::try_new(50.0).expect("valid progress"),
                Speed::ZERO,
            ),
        ],
    )
    .expect("valid world");
    let vehicles = [
        world.vehicle_handle("vehicle-a").expect("vehicle-a"),
        world.vehicle_handle("vehicle-b").expect("vehicle-b"),
    ];

    let points = [point(0.0, 0.0, 0.0), point(100.0, 0.0, 0.0)];
    let spatial = SpatialRegistry::try_new(
        &graph,
        CanonicalFrameId::try_new("campus/main").expect("valid frame"),
        [SpatialEdgeInput::new(edge, &points)],
    )
    .expect("valid spatial registry");

    Fixture {
        spatial,
        parking,
        edge,
        space,
        vehicles,
    }
}

fn close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= 8.0 * f32::EPSILON,
        "actual={actual:?}, expected={expected:?}"
    );
}

#[test]
fn extracts_lane_and_parking_records_in_stable_order() {
    let fixture = fixture();
    let old_token = FramePlacementToken::new(7);
    let new_token = FramePlacementToken::new(8);
    let mut output =
        CanonicalPoseBatchF32::with_capacity(fixture.spatial.frame_id().clone(), old_token, 2);
    let mut scratch = CanonicalPoseBatchScratch::with_capacity(2);
    let inputs = [
        PoseInputRecord::lane(
            fixture.vehicles[0],
            fixture.edge,
            EdgeProgress::try_new(10.0).expect("valid progress"),
        ),
        PoseInputRecord::parking(fixture.vehicles[1], fixture.space),
    ];

    fixture
        .spatial
        .extract_pose_batch(
            &fixture.parking,
            new_token,
            &inputs,
            &mut output,
            &mut scratch,
        )
        .expect("valid batch");

    assert_eq!(output.frame_id(), fixture.spatial.frame_id());
    assert_eq!(output.placement_token(), new_token);
    assert_eq!(output.len(), 2);
    assert!(!output.is_empty());
    assert!(scratch.is_empty());
    assert!(output.capacity() >= 2);
    assert!(scratch.capacity() >= 2);

    let lane = output.records()[0];
    assert_eq!(lane.vehicle(), fixture.vehicles[0]);
    assert_eq!(
        [
            lane.pose().position().x(),
            lane.pose().position().y(),
            lane.pose().position().z(),
        ],
        [10.0, 0.0, 0.0]
    );

    let parked = output.records()[1];
    assert_eq!(parked.vehicle(), fixture.vehicles[1]);
    close(parked.pose().position().x(), 25.0);
    close(parked.pose().position().y(), 0.0);
    close(parked.pose().position().z(), -2.0);
    close(parked.pose().tangent().x(), 0.0);
    close(parked.pose().tangent().y(), 0.0);
    close(parked.pose().tangent().z(), -1.0);
    close(parked.pose().up().x(), 0.0);
    close(parked.pose().up().y(), 1.0);
    close(parked.pose().up().z(), 0.0);
}

#[test]
fn middle_record_failure_preserves_committed_batch_and_old_token() {
    let fixture = fixture();
    let committed_token = FramePlacementToken::new(11);
    let mut output = CanonicalPoseBatchF32::with_capacity(
        fixture.spatial.frame_id().clone(),
        FramePlacementToken::new(10),
        2,
    );
    let mut scratch = CanonicalPoseBatchScratch::with_capacity(2);
    fixture
        .spatial
        .extract_pose_batch(
            &fixture.parking,
            committed_token,
            &[PoseInputRecord::lane(
                fixture.vehicles[0],
                fixture.edge,
                EdgeProgress::try_new(10.0).expect("valid progress"),
            )],
            &mut output,
            &mut scratch,
        )
        .expect("initial batch");
    let old_output = output.clone();

    let error = fixture
        .spatial
        .extract_pose_batch(
            &fixture.parking,
            FramePlacementToken::new(12),
            &[
                PoseInputRecord::lane(
                    fixture.vehicles[0],
                    fixture.edge,
                    EdgeProgress::try_new(20.0).expect("valid progress"),
                ),
                PoseInputRecord::lane(
                    fixture.vehicles[1],
                    fixture.edge,
                    EdgeProgress::try_new(101.0).expect("valid non-negative progress"),
                ),
            ],
            &mut output,
            &mut scratch,
        )
        .expect_err("second record exceeds edge length");

    match error {
        SpatialError::PoseRecordFailed {
            input_index,
            vehicle,
            source,
        } => {
            assert_eq!(input_index, 1);
            assert_eq!(vehicle, fixture.vehicles[1]);
            assert!(matches!(*source, SpatialError::ProgressOutOfRange { .. }));
        }
        other => panic!("unexpected error: {other:?}"),
    }
    assert_eq!(output, old_output);
    assert_eq!(output.placement_token(), committed_token);
    assert!(scratch.is_empty());
}

#[test]
fn frame_mismatch_precedes_record_validation() {
    let fixture = fixture();
    let token = FramePlacementToken::new(21);
    let mut output = CanonicalPoseBatchF32::new(
        CanonicalFrameId::try_new("other/frame").expect("valid other frame"),
        token,
    );
    let old_output = output.clone();
    let mut scratch = CanonicalPoseBatchScratch::new();

    let error = fixture
        .spatial
        .extract_pose_batch(
            &fixture.parking,
            FramePlacementToken::new(22),
            &[PoseInputRecord::lane(
                fixture.vehicles[0],
                fixture.edge,
                EdgeProgress::try_new(101.0).expect("valid non-negative progress"),
            )],
            &mut output,
            &mut scratch,
        )
        .expect_err("frame mismatch must fail first");

    assert_eq!(
        error,
        SpatialError::BatchFrameMismatch {
            registry_frame_id: "campus/main".to_owned(),
            output_frame_id: "other/frame".to_owned(),
        }
    );
    assert_eq!(output, old_output);
    assert!(scratch.is_empty());
}

#[test]
fn parking_pose_rejects_position_outside_canonical_frame_atomically() {
    let fixture = fixture();
    let graph = LaneGraph::try_new([LaneEdge::new(
        "vertical-z",
        EdgeLength::try_new(10.0).expect("valid edge length"),
        std::iter::empty::<&str>(),
    )])
    .expect("valid graph");
    let edge = graph.edge_handle("vertical-z").expect("edge handle");
    let parking = ParkingRegistry::try_new(
        &graph,
        std::iter::empty(),
        [ParkingSpace::new(
            "outside",
            None,
            "vertical-z",
            5.0,
            "vertical-z",
            5.0,
            ParkingSpaceGeometry::new(1.0, 0.0, 5.0, 2.4),
        )],
    )
    .expect("valid parking registry");
    let space = parking.space_handle("outside").expect("space handle");
    let points = [point(16_384.0, 0.0, 0.0), point(16_384.0, 0.0, 10.0)];
    let spatial = SpatialRegistry::try_new(
        &graph,
        CanonicalFrameId::try_new("boundary").expect("valid frame"),
        [SpatialEdgeInput::new(edge, &points)],
    )
    .expect("valid spatial registry");
    let token = FramePlacementToken::new(31);
    let mut output = CanonicalPoseBatchF32::new(spatial.frame_id().clone(), token);
    let old_output = output.clone();
    let mut scratch = CanonicalPoseBatchScratch::with_capacity(1);

    let error = spatial
        .extract_pose_batch(
            &parking,
            FramePlacementToken::new(32),
            &[PoseInputRecord::parking(fixture.vehicles[0], space)],
            &mut output,
            &mut scratch,
        )
        .expect_err("lateral offset leaves canonical frame");

    match error {
        SpatialError::PoseRecordFailed { source, .. } => match *source {
            SpatialError::ParkingPoseComputation { source, .. } => assert!(matches!(
                *source,
                SpatialError::PointComponentOutOfRange {
                    axis: SpatialAxis::X,
                    ..
                }
            )),
            other => panic!("unexpected source: {other:?}"),
        },
        other => panic!("unexpected error: {other:?}"),
    }
    assert_eq!(output, old_output);
    assert!(scratch.is_empty());
}

#[test]
fn unknown_parking_space_is_reported_with_record_context() {
    let fixture = fixture();
    let graph = LaneGraph::try_new([LaneEdge::new(
        "edge",
        EdgeLength::try_new(100.0).expect("valid edge length"),
        std::iter::empty::<&str>(),
    )])
    .expect("valid graph");
    let other_parking = ParkingRegistry::try_new(
        &graph,
        std::iter::empty(),
        [
            ParkingSpace::new(
                "first",
                None,
                "edge",
                20.0,
                "edge",
                20.0,
                ParkingSpaceGeometry::new(2.0, 0.0, 5.0, 2.4),
            ),
            ParkingSpace::new(
                "second",
                None,
                "edge",
                30.0,
                "edge",
                30.0,
                ParkingSpaceGeometry::new(2.0, 0.0, 5.0, 2.4),
            ),
        ],
    )
    .expect("valid other parking registry");
    let unknown = other_parking
        .space_handle("second")
        .expect("second space handle");
    let mut output = CanonicalPoseBatchF32::new(
        fixture.spatial.frame_id().clone(),
        FramePlacementToken::new(40),
    );
    let mut scratch = CanonicalPoseBatchScratch::new();

    let error = fixture
        .spatial
        .extract_pose_batch(
            &fixture.parking,
            FramePlacementToken::new(41),
            &[PoseInputRecord::parking(fixture.vehicles[0], unknown)],
            &mut output,
            &mut scratch,
        )
        .expect_err("foreign space handle index is unknown");

    match error {
        SpatialError::PoseRecordFailed {
            input_index,
            vehicle,
            source,
        } => {
            assert_eq!(input_index, 0);
            assert_eq!(vehicle, fixture.vehicles[0]);
            assert_eq!(
                *source,
                SpatialError::UnknownParkingSpaceHandle { space: unknown }
            );
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn unknown_edge_is_reported_with_record_context() {
    let fixture = fixture();
    let other_graph = LaneGraph::try_new([
        LaneEdge::new(
            "first",
            EdgeLength::try_new(100.0).expect("valid edge length"),
            ["second"],
        ),
        LaneEdge::new(
            "second",
            EdgeLength::try_new(100.0).expect("valid edge length"),
            std::iter::empty::<&str>(),
        ),
    ])
    .expect("valid other graph");
    let unknown = other_graph
        .edge_handle("second")
        .expect("second edge handle");
    let mut output = CanonicalPoseBatchF32::new(
        fixture.spatial.frame_id().clone(),
        FramePlacementToken::new(45),
    );
    let mut scratch = CanonicalPoseBatchScratch::new();

    let error = fixture
        .spatial
        .extract_pose_batch(
            &fixture.parking,
            FramePlacementToken::new(46),
            &[PoseInputRecord::lane(
                fixture.vehicles[0],
                unknown,
                EdgeProgress::ZERO,
            )],
            &mut output,
            &mut scratch,
        )
        .expect_err("foreign edge handle index is unknown");

    match error {
        SpatialError::PoseRecordFailed {
            input_index,
            vehicle,
            source,
        } => {
            assert_eq!(input_index, 0);
            assert_eq!(vehicle, fixture.vehicles[0]);
            assert_eq!(*source, SpatialError::UnknownEdgeHandle { edge: unknown });
        }
        other => panic!("unexpected error: {other:?}"),
    }
    assert!(output.is_empty());
    assert!(scratch.is_empty());
}

#[test]
fn parking_anchors_near_both_edge_endpoints_are_sampled() {
    const ENDPOINT_CLEARANCE_METERS: f64 = 1.0e-9;

    let fixture = fixture();
    let graph = LaneGraph::try_new([LaneEdge::new(
        "edge",
        EdgeLength::try_new(100.0).expect("valid edge length"),
        std::iter::empty::<&str>(),
    )])
    .expect("valid graph");
    let edge = graph.edge_handle("edge").expect("edge handle");
    let lower = ENDPOINT_CLEARANCE_METERS.next_up();
    let upper = (100.0 - ENDPOINT_CLEARANCE_METERS).next_down();
    let parking = ParkingRegistry::try_new(
        &graph,
        std::iter::empty(),
        [
            ParkingSpace::new(
                "lower",
                None,
                "edge",
                lower,
                "edge",
                lower,
                ParkingSpaceGeometry::new(1.0, 0.0, 5.0, 2.4),
            ),
            ParkingSpace::new(
                "upper",
                None,
                "edge",
                upper,
                "edge",
                upper,
                ParkingSpaceGeometry::new(1.0, 0.0, 5.0, 2.4),
            ),
        ],
    )
    .expect("anchors immediately inside both endpoint clearances are valid");
    let points = [point(0.0, 0.0, 0.0), point(100.0, 0.0, 0.0)];
    let spatial = SpatialRegistry::try_new(
        &graph,
        CanonicalFrameId::try_new("endpoint-boundary").expect("valid frame"),
        [SpatialEdgeInput::new(edge, &points)],
    )
    .expect("valid spatial registry");
    let mut output =
        CanonicalPoseBatchF32::new(spatial.frame_id().clone(), FramePlacementToken::new(47));
    let mut scratch = CanonicalPoseBatchScratch::with_capacity(2);

    spatial
        .extract_pose_batch(
            &parking,
            FramePlacementToken::new(48),
            &[
                PoseInputRecord::parking(
                    fixture.vehicles[0],
                    parking.space_handle("lower").expect("lower handle"),
                ),
                PoseInputRecord::parking(
                    fixture.vehicles[1],
                    parking.space_handle("upper").expect("upper handle"),
                ),
            ],
            &mut output,
            &mut scratch,
        )
        .expect("both valid endpoint-adjacent anchors sample");

    close(output.records()[0].pose().position().x(), lower as f32);
    close(output.records()[1].pose().position().x(), upper as f32);
    assert!(scratch.is_empty());
}

#[test]
fn repeated_extraction_is_deterministic_and_reuses_capacity() {
    let fixture = fixture();
    let token = FramePlacementToken::new(49);
    let inputs = [
        PoseInputRecord::lane(
            fixture.vehicles[0],
            fixture.edge,
            EdgeProgress::try_new(10.0).expect("valid progress"),
        ),
        PoseInputRecord::parking(fixture.vehicles[1], fixture.space),
    ];
    let mut output = CanonicalPoseBatchF32::with_capacity(
        fixture.spatial.frame_id().clone(),
        token,
        inputs.len(),
    );
    let mut scratch = CanonicalPoseBatchScratch::with_capacity(inputs.len());

    fixture
        .spatial
        .extract_pose_batch(&fixture.parking, token, &inputs, &mut output, &mut scratch)
        .expect("first extraction");
    let first = output.clone();
    let output_capacity = output.capacity();
    let scratch_capacity = scratch.capacity();

    fixture
        .spatial
        .extract_pose_batch(&fixture.parking, token, &inputs, &mut output, &mut scratch)
        .expect("second extraction");

    assert_eq!(output, first);
    assert_eq!(output.capacity(), output_capacity);
    assert_eq!(scratch.capacity(), scratch_capacity);
    assert!(scratch.is_empty());
}

#[test]
fn fake_host_rejects_batch_from_stale_placement_token() {
    #[derive(Debug, PartialEq)]
    struct HostTransform {
        vehicle: VehicleHandle,
        translation: [f32; 3],
        forward: [f32; 3],
        up: [f32; 3],
    }

    struct FakeHost {
        current_token: FramePlacementToken,
    }

    impl FakeHost {
        fn apply(&self, batch: &CanonicalPoseBatchF32) -> Option<Vec<HostTransform>> {
            if batch.placement_token() != self.current_token {
                return None;
            }
            Some(
                batch
                    .records()
                    .iter()
                    .map(|record| {
                        let pose = record.pose();
                        HostTransform {
                            vehicle: record.vehicle(),
                            translation: [
                                pose.position().x(),
                                pose.position().y(),
                                pose.position().z(),
                            ],
                            forward: [pose.tangent().x(), pose.tangent().y(), pose.tangent().z()],
                            up: [pose.up().x(), pose.up().y(), pose.up().z()],
                        }
                    })
                    .collect(),
            )
        }
    }

    let fixture = fixture();
    let initial_token = FramePlacementToken::new(50);
    let mut host = FakeHost {
        current_token: initial_token,
    };
    let mut output = CanonicalPoseBatchF32::new(fixture.spatial.frame_id().clone(), initial_token);
    let mut scratch = CanonicalPoseBatchScratch::with_capacity(1);
    let inputs = [PoseInputRecord::lane(
        fixture.vehicles[0],
        fixture.edge,
        EdgeProgress::try_new(10.0).expect("valid progress"),
    )];
    fixture
        .spatial
        .extract_pose_batch(
            &fixture.parking,
            initial_token,
            &inputs,
            &mut output,
            &mut scratch,
        )
        .expect("initial batch");
    let applied = host.apply(&output).expect("current batch is accepted");
    assert_eq!(applied.len(), 1);
    assert_eq!(applied[0].vehicle, fixture.vehicles[0]);
    assert_eq!(applied[0].translation, [10.0, 0.0, 0.0]);
    assert_eq!(applied[0].forward, [1.0, 0.0, 0.0]);
    assert_eq!(applied[0].up, [0.0, 1.0, 0.0]);

    host.current_token = FramePlacementToken::new(51);
    assert!(host.apply(&output).is_none());
    fixture
        .spatial
        .extract_pose_batch(
            &fixture.parking,
            host.current_token,
            &inputs,
            &mut output,
            &mut scratch,
        )
        .expect("replacement batch");
    assert!(host.apply(&output).is_some());
    assert_eq!(output.placement_token(), FramePlacementToken::new(51));
}
