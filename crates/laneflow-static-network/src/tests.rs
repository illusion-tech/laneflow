use std::sync::{Arc, atomic::AtomicBool};

use laneflow_format::{FormatLimits, RegistryCheckedFieldValue, check_canonical_network_input};
use laneflow_static_contract::{
    AuthoringLaneOrdinal, EntityKind, LaneEdgeKind, LaneEdgeOrdinal, ManeuverPathOrdinal,
    ParticipantClassOrdinal, RoadCorridorOrdinal, RoadSectionOrdinal, SignalControllerOrdinal,
    SignalPhaseOrdinal,
};

use crate::{
    AccessCell, BuildError, BuildStructure, CorridorElement, FacilityKind,
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};

const MIN_HEADLESS: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfca-variants/min-headless.lfca"
);
const FULL_SPATIAL: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfca"
);
const REORDER_EQUIVALENT: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfca-variants/reorder-equivalent.lfca"
);

const BUILD_LIMITS: SharedNetworkBuildLimits =
    SharedNetworkBuildLimits::new(64 * 1024 * 1024, 16 * 1024 * 1024);

fn build(bytes: &[u8], spatial: SpatialBuildOption) -> Arc<crate::SharedNetworkRevision> {
    let input = check_canonical_network_input(bytes, FormatLimits::HARD)
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
        revision.traffic().lane_lengths_millimetres().len(),
        usize::try_from(lane_count).expect("lane count")
    );
    assert_eq!(
        revision.planning_hints().edge_boundary_weights().len(),
        usize::try_from(lane_count).expect("lane count")
    );

    let ordinal_for_length = |length| {
        let index = revision
            .traffic()
            .lane_lengths_millimetres()
            .iter()
            .position(|actual| *actual == length)
            .expect("fixture lane length");
        LaneEdgeOrdinal::try_from_usize(index).expect("fixture lane ordinal")
    };
    let first = ordinal_for_length(10_000);
    let middle = ordinal_for_length(8_000);
    let last = ordinal_for_length(12_000);
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

    let relations = revision.traffic().relations();
    let corridor_count = revision
        .traffic()
        .entity_counts()
        .count(EntityKind::RoadCorridor);
    if corridor_count > 0 {
        let corridor = RoadCorridorOrdinal::from_raw(0);
        assert!(relations.corridor_elements(corridor).is_some());
        assert!(relations.corridor_reference_section(corridor).is_some());
    }
    for raw in 0..lane_count {
        let edge = LaneEdgeOrdinal::from_raw(raw);
        let _ = relations.lane_edge_authoring_lane(edge);
        let _ = relations.lane_edge_junction(edge);
        let _ = relations.stop_line_for_edge(edge);
    }
    if revision
        .traffic()
        .entity_counts()
        .count(EntityKind::SignalPhase)
        > 0
    {
        use laneflow_static_contract::SignalPhaseOrdinal;
        assert!(
            relations
                .phase_end_offset_ms(SignalPhaseOrdinal::from_raw(0))
                .is_some()
        );
    }
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
    let input =
        check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD).expect("checked input");
    assert!(matches!(
        build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::RetainAvailable,
                SharedNetworkBuildLimits::new(1, u64::MAX),
            ),
        ),
        Err(BuildError::BudgetExceeded {
            structure: BuildStructure::RetainedOutput,
            ..
        })
    ));

    let required =
        build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable).retained_logical_bytes();
    let below_exact =
        check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD).expect("checked input");
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
            ..
        })
    ));

    let exact =
        check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD).expect("checked input");
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
    let input =
        check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD).expect("checked input");
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

    let exact =
        check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD).expect("checked input");
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
fn omit_spatial_endpoint_scratch_is_budgeted_exactly() {
    let input =
        check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD).expect("checked input");
    let result = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::Omit,
            SharedNetworkBuildLimits::new(u64::MAX, 1),
        ),
    );
    let required = match result {
        Err(BuildError::BudgetExceeded {
            structure: BuildStructure::BuilderScratch,
            required,
            ..
        }) => required,
        _ => panic!("omit scratch budget should include spatial join endpoints"),
    };

    let exact =
        check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD).expect("checked input");
    assert!(
        build_shared_network_revision(
            exact,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::Omit,
                SharedNetworkBuildLimits::new(u64::MAX, required),
            ),
        )
        .is_ok()
    );
}

#[test]
fn pre_cancelled_build_returns_no_root() {
    let input =
        check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD).expect("checked input");
    let cancelled = AtomicBool::new(true);
    let result = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, BUILD_LIMITS)
            .with_cancellation(&cancelled),
    );
    assert!(matches!(result, Err(BuildError::Cancelled)));
}

fn ordinals(row: laneflow_format::RegistryCheckedRowView<'_>, tag: u16) -> Vec<u32> {
    match row
        .field_by_tag(tag)
        .expect("field")
        .value()
        .expect("value")
    {
        RegistryCheckedFieldValue::OrdinalVectorU32(values) => (0..values.len())
            .map(|index| values.get(index).expect("member"))
            .collect(),
        _ => panic!("expected ordinal vector"),
    }
}

fn u64_field(row: laneflow_format::RegistryCheckedRowView<'_>, tag: u16) -> u64 {
    match row
        .field_by_tag(tag)
        .expect("field")
        .value()
        .expect("value")
    {
        RegistryCheckedFieldValue::U64(value) => value,
        _ => panic!("expected u64"),
    }
}

#[test]
fn full_spatial_preserves_section_lane_and_controller_phase_sequence() {
    let input =
        check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD).expect("checked input");
    let view = input.value_checked_view();
    let entities = view.registry_view().section(2).expect("entities");
    let revision = build(FULL_SPATIAL, SpatialBuildOption::Omit);
    let relations = revision.traffic().relations();

    let section_table = entities.table(1).expect("RoadSection");
    for (index, row) in section_table.rows().enumerate() {
        let expected = ordinals(row, 5)
            .into_iter()
            .map(AuthoringLaneOrdinal::from_raw)
            .collect::<Vec<_>>();
        let section = RoadSectionOrdinal::from_raw(u32::try_from(index).expect("fits"));
        assert_eq!(
            relations.section_lanes(section).expect("section lanes"),
            expected.as_slice()
        );
    }

    let controller_table = entities.table(11).expect("SignalController");
    for (index, row) in controller_table.rows().enumerate() {
        let controller = SignalControllerOrdinal::from_raw(u32::try_from(index).expect("fits"));
        let expected = ordinals(row, 6)
            .into_iter()
            .map(SignalPhaseOrdinal::from_raw)
            .collect::<Vec<_>>();
        assert_eq!(
            relations
                .controller_phases(controller)
                .expect("controller phases"),
            expected.as_slice()
        );
        let cycle = u64_field(row, 4);
        assert_eq!(relations.controller_cycle_ms(controller), Some(cycle));
        let mut cursor = 0_u64;
        for phase in &expected {
            let duration = relations.phase_duration_ms(*phase).expect("duration");
            cursor += duration;
            assert_eq!(relations.phase_end_offset_ms(*phase), Some(cursor));
        }
        if let Some(last) = expected.last() {
            assert_eq!(relations.phase_end_offset_ms(*last), Some(cycle));
        }
    }
}

#[test]
fn full_spatial_closes_corridor_facility_kind_and_uncovered_edges() {
    let revision = build(FULL_SPATIAL, SpatialBuildOption::Omit);
    let relations = revision.traffic().relations();
    let counts = revision.traffic().entity_counts();
    let corridor_count = counts.count(EntityKind::RoadCorridor);
    let mut saw_section = false;
    let mut saw_band = false;
    for raw in 0..corridor_count {
        let corridor = RoadCorridorOrdinal::from_raw(raw);
        let elements = relations.corridor_elements(corridor).expect("elements");
        for element in elements {
            match element {
                CorridorElement::RoadSection(_) => saw_section = true,
                CorridorElement::FacilityBand(_) => saw_band = true,
            }
        }
        assert!(relations.corridor_reference_section(corridor).is_some());
    }
    if corridor_count > 0 {
        assert!(saw_section || saw_band);
    }

    let section_count = counts.count(EntityKind::RoadSection);
    for raw in 0..section_count {
        let kind = relations
            .section_kind(RoadSectionOrdinal::from_raw(raw))
            .expect("section kind");
        if matches!(kind, FacilityKind::Custom { .. }) {
            assert!(relations.facility_kind_token(kind).is_some());
        } else {
            assert!(matches!(
                relations.facility_kind_token(kind),
                Some("motorLane")
                    | Some("nonMotorLane")
                    | Some("sidewalk")
                    | Some("median")
                    | Some("plantingStrip")
                    | Some("facilityStrip")
                    | Some("shoulder")
            ));
        }
    }

    let lane_count = counts.count(EntityKind::LaneEdge);
    for raw in 0..lane_count {
        let edge = LaneEdgeOrdinal::from_raw(raw);
        let _ = relations.lane_edge_authoring_lane(edge);
        let _ = relations.lane_edge_junction(edge);
        let _ = relations.stop_line_for_edge(edge);
    }
}

#[test]
fn full_spatial_access_cells_do_not_scan_and_stay_in_rule_bounds() {
    let revision = build(FULL_SPATIAL, SpatialBuildOption::Omit);
    let relations = revision.traffic().relations();
    let counts = revision.traffic().entity_counts();
    let class_count = counts.count(EntityKind::ParticipantClass);
    let rule_count = counts.count(EntityKind::AccessRule);
    let lane_count = counts.count(EntityKind::LaneEdge);
    for edge in 0..lane_count {
        for class in 0..class_count {
            match relations.edge_access(
                LaneEdgeOrdinal::from_raw(edge),
                ParticipantClassOrdinal::from_raw(class),
            ) {
                None => panic!("in-range access query must be Some"),
                Some(AccessCell::Unconstrained) => {}
                Some(AccessCell::Decided { rule, .. }) => {
                    assert!(rule.raw() < rule_count);
                }
            }
        }
    }
    assert!(
        relations
            .edge_access(
                LaneEdgeOrdinal::from_raw(lane_count),
                ParticipantClassOrdinal::from_raw(0)
            )
            .is_none()
            || class_count == 0 && lane_count == 0
    );
    for path in 0..counts.count(EntityKind::ManeuverPath) {
        for class in 0..class_count {
            match relations.path_access(
                ManeuverPathOrdinal::from_raw(path),
                ParticipantClassOrdinal::from_raw(class),
            ) {
                None => panic!("in-range access query must be Some"),
                Some(AccessCell::Unconstrained) => {}
                Some(AccessCell::Decided { rule, .. }) => {
                    assert!(rule.raw() < rule_count);
                }
            }
        }
    }
}

#[test]
fn full_spatial_has_no_static_route_entity_table() {
    let input =
        check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD).expect("checked input");
    let view = input.value_checked_view();
    let entities = view.registry_view().section(2).expect("entities");
    for table in entities.tables() {
        assert_ne!(table.kind(), EntityKind::StaticRoute.code());
    }
    let _ = build(FULL_SPATIAL, SpatialBuildOption::Omit);
}

#[test]
fn full_spatial_entity_views_cover_required_columns() {
    let revision = build(FULL_SPATIAL, SpatialBuildOption::Omit);
    let relations = revision.traffic().relations();
    let counts = revision.traffic().entity_counts();
    for raw in 0..counts.count(EntityKind::ManeuverGate) {
        assert!(
            relations
                .maneuver_gate(laneflow_static_contract::ManeuverGateOrdinal::from_raw(raw))
                .is_some()
        );
    }
    for raw in 0..counts.count(EntityKind::WaitingZone) {
        assert!(
            relations
                .waiting_zone(laneflow_static_contract::WaitingZoneOrdinal::from_raw(raw))
                .is_some()
        );
    }
    for raw in 0..counts.count(EntityKind::SignalController) {
        let view = relations
            .signal_controller(SignalControllerOrdinal::from_raw(raw))
            .expect("controller view");
        assert_eq!(
            view.cycle_ms(),
            relations
                .controller_cycle_ms(SignalControllerOrdinal::from_raw(raw))
                .expect("cycle")
        );
    }
    for raw in 0..counts.count(EntityKind::ParkingSpace) {
        let view = relations
            .parking_space(laneflow_static_contract::ParkingSpaceOrdinal::from_raw(raw))
            .expect("parking view");
        let _ = view.exit();
        let _ = view.entry();
    }
    for raw in 0..counts.count(EntityKind::VehicleProfile) {
        assert!(
            relations
                .vehicle_profile(laneflow_static_contract::VehicleProfileOrdinal::from_raw(
                    raw
                ))
                .is_some()
        );
    }
    for raw in 0..counts.count(EntityKind::ParticipantClass) {
        assert!(
            relations
                .participant_class(ParticipantClassOrdinal::from_raw(raw))
                .is_some()
        );
    }
    for raw in 0..counts.count(EntityKind::RoadSection) {
        let section = RoadSectionOrdinal::from_raw(raw);
        let corridor = relations.section_corridor(section).expect("section owner");
        assert!(
            relations
                .corridor_elements(corridor)
                .expect("corridor elements")
                .contains(&CorridorElement::RoadSection(section))
        );
    }
    for raw in 0..counts.count(EntityKind::AuthoringLane) {
        let lane = AuthoringLaneOrdinal::from_raw(raw);
        let section = relations
            .authoring_section(lane)
            .expect("authoring section");
        assert!(
            relations
                .section_lanes(section)
                .expect("section lanes")
                .contains(&lane)
        );
    }
    for raw in 0..counts.count(EntityKind::Movement) {
        assert!(
            relations
                .movement_junction(laneflow_static_contract::MovementOrdinal::from_raw(raw))
                .is_some()
        );
    }
    for raw in 0..counts.count(EntityKind::LaneGroup) {
        assert!(
            relations
                .lane_group_section(laneflow_static_contract::LaneGroupOrdinal::from_raw(raw))
                .is_some()
        );
    }
    for raw in 0..counts.count(EntityKind::FacilityBand) {
        assert!(
            relations
                .band_corridor(laneflow_static_contract::FacilityBandOrdinal::from_raw(raw))
                .is_some()
        );
    }
}
