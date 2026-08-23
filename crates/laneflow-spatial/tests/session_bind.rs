use std::sync::Arc;

use laneflow_format::{FormatLimits, check_canonical_network_input_v1};
use laneflow_spatial::{
    CanonicalPoseBatch, FramePlacementToken, PoseInput, PoseRecordId, SpatialSession,
};
use laneflow_static_contract::LaneEdgeOrdinal;
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
fn bind_full_spatial_and_extract_lane_pose() {
    let revision = revision();
    assert!(Arc::ptr_eq(
        &revision,
        &SpatialSession::bind(Arc::clone(&revision))
            .expect("bind")
            .expect("session")
            .revision()
    ));
    let mut session = SpatialSession::bind(revision)
        .expect("bind")
        .expect("session");
    let edge = LaneEdgeOrdinal::from_raw(0);
    let mut output = CanonicalPoseBatch::new();
    session
        .extract_pose_batch(
            FramePlacementToken::new(1),
            &[PoseInput::lane(PoseRecordId::new(7), edge, 0.0)],
            &mut output,
        )
        .expect("extract");
    assert_eq!(output.records().len(), 1);
    assert_eq!(output.records()[0].record(), PoseRecordId::new(7));
    assert_eq!(output.placement_token(), FramePlacementToken::new(1));

    let previous_len = output.records().len();
    let failed = session.extract_pose_batch(
        FramePlacementToken::new(2),
        &[
            PoseInput::lane(PoseRecordId::new(1), edge, 0.0),
            PoseInput::lane(PoseRecordId::new(2), edge, 1.0e9),
        ],
        &mut output,
    );
    assert!(failed.is_err());
    assert_eq!(output.records().len(), previous_len);
    assert_eq!(output.placement_token(), FramePlacementToken::new(1));
}
