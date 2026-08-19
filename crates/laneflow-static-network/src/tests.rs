use std::sync::{Arc, atomic::AtomicBool};

use laneflow_format::{FormatLimits, check_canonical_network_input_v1};
use laneflow_static_contract::{EntityKind, LaneEdgeKind, LaneEdgeOrdinal};

use crate::{
    BuildError, BuildStructure, SharedNetworkBuildLimits, SharedNetworkBuildOptions,
    SpatialBuildOption, build_shared_network_revision,
};

const MIN_HEADLESS: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable-v1/lfca-v1-variants/min-headless.lfca"
);
const FULL_SPATIAL: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable-v1/lfca-v1-full-spatial/expected.lfca"
);

const BUILD_LIMITS: SharedNetworkBuildLimits =
    SharedNetworkBuildLimits::new(64 * 1024 * 1024, 16 * 1024 * 1024);

fn build(bytes: &[u8], spatial: SpatialBuildOption) -> Arc<crate::SharedNetworkRevision> {
    let input = check_canonical_network_input_v1(bytes, FormatLimits::V1_HARD)
        .expect("checked canonical network input");
    build_shared_network_revision(input, SharedNetworkBuildOptions::new(spatial, BUILD_LIMITS))
        .expect("shared network revision")
}

#[test]
fn minimal_headless_build_has_required_components_and_no_spatial() {
    let revision = build(MIN_HEADLESS, SpatialBuildOption::RetainAvailable);

    assert_eq!(revision.traffic().lane_edge_count(), 0);
    assert_eq!(
        revision.identity().entity_count(EntityKind::LaneEdge),
        revision.traffic().lane_edge_count()
    );
    assert!(revision.planning_hints().edge_boundary_weights().is_empty());
    assert!(revision.spatial().is_none());
    assert!(revision.retained_logical_bytes() > 0);
}

#[test]
fn full_spatial_build_closes_identity_lane_csr_and_lane_pose() {
    let revision = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
    let lane_count = revision.traffic().lane_edge_count();
    assert!(lane_count > 0);
    assert_eq!(
        revision.traffic().lane_lengths_meters().len(),
        usize::try_from(lane_count).expect("lane count")
    );
    assert_eq!(
        revision.planning_hints().edge_boundary_weights().len(),
        usize::try_from(lane_count).expect("lane count")
    );

    let first = LaneEdgeOrdinal::from_raw(0);
    let stable_id = revision
        .identity()
        .stable_id::<LaneEdgeKind>(first)
        .expect("lane identity");
    assert_eq!(
        revision.identity().ordinal(stable_id),
        Some(LaneEdgeOrdinal::from_raw(0))
    );
    assert!(revision.traffic().successors(first).is_some());
    assert!(revision.traffic().predecessors(first).is_some());

    let spatial = revision.spatial().expect("spatial component");
    let lane_pose = spatial.lane_pose().expect("lane pose capability");
    assert_eq!(lane_pose.lane_edge_count(), lane_count);
    let geometry = lane_pose.lane_geometry(first).expect("first lane geometry");
    assert!(geometry.points().len() >= 2);
    assert_eq!(geometry.segments().len() + 1, geometry.points().len());
}

#[test]
fn omit_validates_but_retains_no_spatial_payload() {
    let revision = build(FULL_SPATIAL, SpatialBuildOption::Omit);
    assert!(revision.spatial().is_none());
    assert!(revision.traffic().lane_edge_count() > 0);
}

#[test]
fn successful_root_outlives_input_bytes_and_arc_clones_share_components() {
    let revision = {
        let owned = FULL_SPATIAL.to_vec();
        build(&owned, SpatialBuildOption::Omit)
    };
    let clones: Vec<_> = (0..32).map(|_| Arc::clone(&revision)).collect();
    assert!(clones.iter().all(|clone| Arc::ptr_eq(&revision, clone)));
    assert!(revision.traffic().lane_edge_count() > 0);
}

#[test]
fn retained_and_scratch_limits_fail_before_a_root_exists() {
    let input = check_canonical_network_input_v1(FULL_SPATIAL, FormatLimits::V1_HARD)
        .expect("checked input");
    let result = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(1, 1),
        ),
    );
    assert!(matches!(
        result,
        Err(BuildError::BudgetExceeded {
            structure: BuildStructure::RetainedOutput,
            ..
        })
    ));
}

#[test]
fn pre_cancelled_build_returns_no_root() {
    let input = check_canonical_network_input_v1(FULL_SPATIAL, FormatLimits::V1_HARD)
        .expect("checked input");
    let cancelled = AtomicBool::new(true);
    let result = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, BUILD_LIMITS)
            .with_cancellation(&cancelled),
    );
    assert!(matches!(result, Err(BuildError::Cancelled)));
}
