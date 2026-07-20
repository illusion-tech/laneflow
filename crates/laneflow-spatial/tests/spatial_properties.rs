use std::sync::OnceLock;

use laneflow_core::{EdgeLength, EdgeProgress, LaneEdge, LaneGraph};
use laneflow_spatial::{
    CanonicalFrameId, CanonicalPoint3F32, SPATIAL_JOIN_POSITION_TOLERANCE_METERS,
    SPATIAL_MIN_SEGMENT_LENGTH_METERS, SpatialEdgeInput, SpatialError, SpatialRegistry,
};
use proptest::{
    prop_assert,
    test_runner::{Config, TestRunner},
};

#[path = "support/pose_batch_scale.rs"]
mod scale_support;

use scale_support::{F64CandidateRegistry, RuntimeFixture};

const POSITION_ERROR_BUDGET_METERS: f64 = 0.01;
const DIRECTION_ERROR_BUDGET_DEGREES: f64 = 0.5;

fn oracle_fixture() -> &'static RuntimeFixture {
    static FIXTURE: OnceLock<RuntimeFixture> = OnceLock::new();
    FIXTURE.get_or_init(RuntimeFixture::new)
}

fn oracle_registry() -> &'static F64CandidateRegistry {
    static ORACLE: OnceLock<F64CandidateRegistry> = OnceLock::new();
    ORACLE.get_or_init(|| oracle_fixture().f64_candidate())
}

#[test]
fn canonical_f32_sampling_tracks_f64_oracle_across_the_full_edge() {
    let fixture = oracle_fixture();
    let oracle = oracle_registry();
    let mut runner = TestRunner::new(Config {
        cases: 512,
        failure_persistence: None,
        ..Config::default()
    });

    runner
        .run(&(0_u32..=1_000_000), |fraction| {
            let progress = fixture.core_length * f64::from(fraction) / 1_000_000.0;
            let progress = EdgeProgress::try_new(progress).expect("generated progress is valid");
            let actual = fixture
                .spatial
                .sample(fixture.edge, progress)
                .expect("generated production sample");
            let expected = oracle
                .sample(fixture.edge, progress)
                .expect("generated oracle sample");

            let position = actual.position();
            let expected_position = expected.position();
            let position_error = (f64::from(position.x()) - expected_position[0])
                .hypot(f64::from(position.y()) - expected_position[1])
                .hypot(f64::from(position.z()) - expected_position[2]);
            let tangent_error = direction_error_degrees(
                [
                    f64::from(actual.tangent().x()),
                    f64::from(actual.tangent().y()),
                    f64::from(actual.tangent().z()),
                ],
                expected.tangent(),
            );
            let up_error = direction_error_degrees(
                [
                    f64::from(actual.up().x()),
                    f64::from(actual.up().y()),
                    f64::from(actual.up().z()),
                ],
                expected.up(),
            );

            prop_assert!(position_error.is_finite());
            prop_assert!(tangent_error.is_finite());
            prop_assert!(up_error.is_finite());
            prop_assert!(
                position_error <= POSITION_ERROR_BUDGET_METERS,
                "position error {position_error} m at progress {}",
                progress.value()
            );
            prop_assert!(
                tangent_error <= DIRECTION_ERROR_BUDGET_DEGREES,
                "tangent error {tangent_error} deg at progress {}",
                progress.value()
            );
            prop_assert!(
                up_error <= DIRECTION_ERROR_BUDGET_DEGREES,
                "up error {up_error} deg at progress {}",
                progress.value()
            );
            Ok(())
        })
        .unwrap_or_else(|error| panic!("f64 oracle property failed: {error}"));
}

#[test]
fn endpoint_vertex_and_join_boundaries_are_explicit() {
    let fixture = oracle_fixture();
    let oracle = oracle_registry();
    for progress in [EdgeProgress::ZERO, edge_progress(fixture.core_length)] {
        let actual = fixture
            .spatial
            .sample(fixture.edge, progress)
            .expect("endpoint sample");
        let expected = oracle
            .sample(fixture.edge, progress)
            .expect("endpoint oracle sample");
        assert_position_within_budget(actual.position(), expected.position());
    }

    let below = f32::from_bits(SPATIAL_JOIN_POSITION_TOLERANCE_METERS.to_bits() - 1);
    let exact = SPATIAL_JOIN_POSITION_TOLERANCE_METERS;
    let above = f32::from_bits(SPATIAL_JOIN_POSITION_TOLERANCE_METERS.to_bits() + 1);
    assert!(joined_registry(below).is_ok());
    assert!(joined_registry(exact).is_ok());
    assert!(matches!(
        joined_registry(above),
        Err(SpatialError::DisconnectedEdgeJoin {
            distance_meters,
            tolerance_meters,
            ..
        }) if distance_meters == above
            && tolerance_meters == SPATIAL_JOIN_POSITION_TOLERANCE_METERS
    ));
}

#[test]
fn degenerate_and_near_vertical_boundaries_are_rejected_without_partial_registry() {
    let graph = graph(&[("edge", f64::from(SPATIAL_MIN_SEGMENT_LENGTH_METERS), &[])]);
    let edge = graph.edge_handle("edge").expect("edge handle");
    let exact_min = [
        point(0.0, 0.0, 0.0),
        point(SPATIAL_MIN_SEGMENT_LENGTH_METERS, 0.0, 0.0),
    ];
    assert!(matches!(
        SpatialRegistry::try_new(
            &graph,
            frame_id("degenerate"),
            [SpatialEdgeInput::new(edge, &exact_min)],
        ),
        Err(SpatialError::DegenerateSegment {
            segment_index: 0,
            ..
        })
    ));

    assert!(near_vertical_registry(0.4).is_err());
    assert!(near_vertical_registry(0.6).is_ok());
}

fn joined_registry(offset: f32) -> Result<SpatialRegistry, SpatialError> {
    let a_length = 1.0_f32 + offset;
    let lane_graph = graph(&[("A", f64::from(a_length), &["B"]), ("B", 1.0, &[])]);
    let edge_a = lane_graph.edge_handle("A").expect("edge A");
    let edge_b = lane_graph.edge_handle("B").expect("edge B");
    let a = [point(-1.0, 0.0, 0.0), point(offset, 0.0, 0.0)];
    let b = [point(0.0, 0.0, 0.0), point(1.0, 0.0, 0.0)];
    SpatialRegistry::try_new(
        &lane_graph,
        frame_id("join"),
        [
            SpatialEdgeInput::new(edge_a, &a),
            SpatialEdgeInput::new(edge_b, &b),
        ],
    )
}

fn near_vertical_registry(angle_degrees: f32) -> Result<SpatialRegistry, SpatialError> {
    let angle = angle_degrees.to_radians();
    let horizontal = angle.sin();
    let vertical = angle.cos();
    let length = horizontal.hypot(vertical);
    let lane_graph = graph(&[("edge", f64::from(length), &[])]);
    let edge = lane_graph.edge_handle("edge").expect("edge handle");
    let points = [point(0.0, 0.0, 0.0), point(horizontal, vertical, 0.0)];
    SpatialRegistry::try_new(
        &lane_graph,
        frame_id("near-vertical"),
        [SpatialEdgeInput::new(edge, &points)],
    )
}

fn graph(edges: &[(&str, f64, &[&str])]) -> LaneGraph {
    LaneGraph::try_new(edges.iter().map(|(id, length, next)| {
        LaneEdge::new(
            *id,
            EdgeLength::try_new(*length).expect("valid edge length"),
            next.iter().copied(),
        )
    }))
    .expect("valid graph")
}

fn point(x: f32, y: f32, z: f32) -> CanonicalPoint3F32 {
    CanonicalPoint3F32::try_new(x, y, z).expect("valid point")
}

fn frame_id(suffix: &str) -> CanonicalFrameId {
    CanonicalFrameId::try_new(format!("validation/{suffix}")).expect("valid frame")
}

fn edge_progress(value: f64) -> EdgeProgress {
    EdgeProgress::try_new(value).expect("valid progress")
}

fn direction_error_degrees(actual: [f64; 3], expected: [f64; 3]) -> f64 {
    let dot = actual
        .iter()
        .zip(expected)
        .map(|(left, right)| left * right)
        .sum::<f64>()
        .clamp(-1.0, 1.0);
    dot.acos().to_degrees()
}

fn assert_position_within_budget(actual: CanonicalPoint3F32, expected: [f64; 3]) {
    let error = (f64::from(actual.x()) - expected[0])
        .hypot(f64::from(actual.y()) - expected[1])
        .hypot(f64::from(actual.z()) - expected[2]);
    assert!(error <= POSITION_ERROR_BUDGET_METERS, "error={error}");
}
