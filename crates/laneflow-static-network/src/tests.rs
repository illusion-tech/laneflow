use std::sync::{Arc, atomic::AtomicBool};

use laneflow_format::{FormatLimits, check_canonical_network_input_v1};
use laneflow_static_contract::{EntityKind, LaneEdgeKind, LaneEdgeOrdinal, ManeuverPathOrdinal};

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
const REORDER_EQUIVALENT: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable-v1/lfca-v1-variants/reorder-equivalent.lfca"
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

    let ordinal_for_length = |length| {
        let index = revision
            .traffic()
            .lane_lengths_meters()
            .iter()
            .position(|actual| *actual == length)
            .expect("fixture lane length");
        LaneEdgeOrdinal::try_from_usize(index).expect("fixture lane ordinal")
    };
    let first = ordinal_for_length(10.0);
    let middle = ordinal_for_length(8.0);
    let last = ordinal_for_length(12.0);
    let stable_id = revision
        .identity()
        .stable_id::<LaneEdgeKind>(first)
        .expect("lane identity");
    assert_eq!(revision.identity().ordinal(stable_id), Some(first));
    assert_eq!(
        revision
            .traffic()
            .successors(first)
            .expect("first successors"),
        &[middle]
    );
    assert_eq!(
        revision
            .traffic()
            .predecessors(first)
            .expect("first predecessors"),
        &[]
    );
    assert_eq!(
        revision
            .traffic()
            .successors(middle)
            .expect("middle successors"),
        &[last]
    );
    assert_eq!(
        revision
            .traffic()
            .predecessors(middle)
            .expect("middle predecessors"),
        &[first]
    );
    assert_eq!(
        revision
            .traffic()
            .successors(last)
            .expect("last successors"),
        &[]
    );
    assert_eq!(
        revision
            .traffic()
            .predecessors(last)
            .expect("last predecessors"),
        &[middle]
    );
    let weights = revision.planning_hints().edge_boundary_weights();
    assert_eq!(weights[first.index()], 1);
    assert_eq!(weights[middle.index()], 2);
    assert_eq!(weights[last.index()], 1);

    let maneuvers = revision.traffic().maneuvers();
    assert_eq!(maneuvers.maneuver_path_count(), 1);
    let path_ordinal = ManeuverPathOrdinal::from_raw(0);
    let path = maneuvers
        .maneuver_path(path_ordinal)
        .expect("fixture maneuver path");
    assert_eq!(path.edges(), &[first, middle, last]);
    assert_eq!(path.maneuver_gates().len(), 2);
    assert_eq!(path.waiting_zones().len(), 1);

    let first_candidates = maneuvers
        .transition_candidates(first)
        .expect("first transition candidates");
    assert_eq!(first_candidates.len(), 1);
    assert_eq!(first_candidates[0].successor(), middle);
    assert_eq!(first_candidates[0].maneuver_path(), path_ordinal);
    assert_eq!(first_candidates[0].transition_index(), 0);
    assert_eq!(
        first_candidates[0].maneuver_gate(),
        Some(path.maneuver_gates()[0])
    );

    let middle_candidates = maneuvers
        .transition_candidates(middle)
        .expect("middle transition candidates");
    assert_eq!(middle_candidates.len(), 1);
    assert_eq!(middle_candidates[0].successor(), last);
    assert_eq!(middle_candidates[0].maneuver_path(), path_ordinal);
    assert_eq!(middle_candidates[0].transition_index(), 1);
    assert_eq!(
        middle_candidates[0].maneuver_gate(),
        Some(path.maneuver_gates()[1])
    );
    assert_eq!(
        maneuvers
            .transition_candidates(last)
            .expect("last transition candidates"),
        &[]
    );

    let spatial = revision.spatial().expect("spatial component");
    let lane_pose = spatial.lane_pose().expect("lane pose capability");
    assert_eq!(lane_pose.lane_edge_count(), lane_count);
    let geometry = lane_pose.lane_geometry(first).expect("first lane geometry");
    assert!(geometry.points().len() >= 2);
    assert_eq!(geometry.segments().len() + 1, geometry.points().len());
}

#[test]
fn headless_lane_csr_retains_non_internal_successors_and_predecessors() {
    let revision = build(REORDER_EQUIVALENT, SpatialBuildOption::Omit);
    let lane_count = revision.traffic().lane_edge_count();
    let csr: Vec<(Vec<u32>, Vec<u32>)> = (0..lane_count)
        .map(|raw| {
            let edge = LaneEdgeOrdinal::from_raw(raw);
            let successors = revision
                .traffic()
                .successors(edge)
                .expect("successor range")
                .iter()
                .map(|ordinal| ordinal.raw())
                .collect();
            let predecessors = revision
                .traffic()
                .predecessors(edge)
                .expect("predecessor range")
                .iter()
                .map(|ordinal| ordinal.raw())
                .collect();
            (successors, predecessors)
        })
        .collect();
    assert_eq!(
        csr,
        [
            (vec![1, 2], vec![]),
            (vec![], vec![0]),
            (vec![], vec![0]),
            (vec![4, 5], vec![]),
            (vec![], vec![3]),
            (vec![], vec![3]),
        ]
    );
    assert_eq!(
        revision.planning_hints().edge_boundary_weights(),
        &[2, 1, 1, 2, 1, 1]
    );
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
fn retained_limit_fails_before_a_root_exists_and_exact_boundary_succeeds() {
    let input = check_canonical_network_input_v1(FULL_SPATIAL, FormatLimits::V1_HARD)
        .expect("checked input");
    let result = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(1, u64::MAX),
        ),
    );
    let required = match result {
        Err(BuildError::BudgetExceeded {
            structure: BuildStructure::RetainedOutput,
            required,
            ..
        }) => required,
        _ => panic!("retained budget should fail first"),
    };

    let below_exact = check_canonical_network_input_v1(FULL_SPATIAL, FormatLimits::V1_HARD)
        .expect("checked input");
    assert!(matches!(
        build_shared_network_revision(
            below_exact,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::RetainAvailable,
                SharedNetworkBuildLimits::new(required - 1, u64::MAX),
            ),
        ),
        Err(BuildError::BudgetExceeded {
            structure: BuildStructure::RetainedOutput,
            required: actual,
            ..
        }) if actual == required
    ));

    let exact = check_canonical_network_input_v1(FULL_SPATIAL, FormatLimits::V1_HARD)
        .expect("checked input");
    let root = build_shared_network_revision(
        exact,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(required, u64::MAX),
        ),
    );
    let root = root.expect("exact retained limit");
    assert_eq!(root.retained_logical_bytes(), required);
}

#[test]
fn scratch_limit_fails_before_a_root_exists_and_exact_boundary_succeeds() {
    let input = check_canonical_network_input_v1(FULL_SPATIAL, FormatLimits::V1_HARD)
        .expect("checked input");
    let result = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(u64::MAX, 1),
        ),
    );
    let required = match result {
        Err(BuildError::BudgetExceeded {
            structure: BuildStructure::BuilderScratch,
            required,
            ..
        }) => required,
        _ => panic!("scratch budget should fail after retained budget passes"),
    };

    let exact = check_canonical_network_input_v1(FULL_SPATIAL, FormatLimits::V1_HARD)
        .expect("checked input");
    assert!(
        build_shared_network_revision(
            exact,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::RetainAvailable,
                SharedNetworkBuildLimits::new(u64::MAX, required),
            ),
        )
        .is_ok()
    );
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
