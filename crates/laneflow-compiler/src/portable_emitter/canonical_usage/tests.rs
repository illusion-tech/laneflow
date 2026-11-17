use super::*;
use crate::canonical_usage::lir_statistics;
use crate::compiler::portable_fixture_tests::{
    full_spatial_portable_fixture_candidate, full_spatial_portable_fixture_output,
};
use laneflow_static_contract::CanonicalNetworkUsage;

fn registry_view(bytes: &[u8]) -> RegistryCheckedObjectView<'_> {
    laneflow_format::preflight_object_values(
        bytes,
        laneflow_static_contract::PortableObjectKind::CanonicalArtifact,
        laneflow_format::FormatLimits::HARD,
    )
    .unwrap()
    .registry_view()
}

/// 全空间固定样例按角色的期望出现记录数；未列出的计入角色为 0。
const FULL_SPATIAL_ROLE_COUNTS: [(SourceRelationRole, u64); 29] = [
    (SourceRelationRole::RoadCorridorElement, 6),
    (SourceRelationRole::RoadSectionLane, 5),
    (SourceRelationRole::AuthoringLaneEdge, 7),
    (SourceRelationRole::LaneGroupMember, 1),
    (SourceRelationRole::JunctionMovement, 3),
    (SourceRelationRole::MovementManeuverPath, 3),
    (SourceRelationRole::ManeuverPathEdge, 9),
    (SourceRelationRole::JunctionInternalEdge, 3),
    (SourceRelationRole::ManeuverPathGate, 4),
    (SourceRelationRole::ManeuverPathWaitingZone, 1),
    (SourceRelationRole::StopLineManeuverGate, 4),
    (SourceRelationRole::JunctionConflictZone, 1),
    (SourceRelationRole::JunctionParticipantStream, 2),
    (SourceRelationRole::SignalControllerGroup, 2),
    (SourceRelationRole::SignalControllerPhase, 2),
    (SourceRelationRole::SignalPhaseState, 4),
    (SourceRelationRole::ManeuverGateSignalGroup, 2),
    (SourceRelationRole::ParkingSpaceFacility, 1),
    (SourceRelationRole::ParkingSpaceEntry, 1),
    (SourceRelationRole::ParkingSpaceExit, 1),
    (SourceRelationRole::ParticipantClassExtends, 1),
    (SourceRelationRole::AccessRuleTarget, 1),
    (SourceRelationRole::AccessRuleParticipantClass, 1),
    (SourceRelationRole::VehicleProfileParticipantClass, 1),
    (SourceRelationRole::CanonicalFrameLaneEdgeGeometry, 9),
    (SourceRelationRole::CanonicalFrameFacilityBandGeometry, 1),
    (SourceRelationRole::ParticipantStreamManeuverPath, 2),
    (SourceRelationRole::ParticipantStreamConflictPassage, 2),
    (SourceRelationRole::CanonicalFrameConflictZoneRegion, 1),
];

#[test]
fn full_spatial_fixture_statistics_match_expected_values_on_both_sides() {
    let output = full_spatial_portable_fixture_output();
    let candidate = full_spatial_portable_fixture_candidate();
    let lir = lir_statistics(output.lir().unit());
    let lfca = lfca_statistics(
        registry_view(candidate.canonical_artifact().bytes()),
        PortableEmissionError::InternalBindingMismatch,
    )
    .unwrap();

    for statistics in [lir, lfca] {
        for role in SourceRelationRole::ALL {
            let expected = FULL_SPATIAL_ROLE_COUNTS
                .iter()
                .find(|(listed, _)| *listed == role)
                .map_or(0, |(_, count)| *count);
            assert_eq!(statistics.relation_roles.get(role), expected, "{role:?}");
        }
        assert_eq!(
            statistics.usage(),
            CanonicalNetworkUsage::new(59, 4, 1, 81, 24)
        );
    }
    assert_eq!(output.lir().canonical_network_usage(), lir.usage());
}

#[test]
fn canonical_relation_count_extends_relation_tuples_with_states_and_geometry_records() {
    // 迁移核对：容量计数等于 Relation 通道元组数加信号状态记录与三种几何记录数。
    let output = full_spatial_portable_fixture_output();
    let lir = output.lir().unit();
    let statistics = lir_statistics(lir);
    let tuples = super::super::relations::canonical_relation_count(lir) as u64;
    let roles = statistics.relation_roles;
    assert_eq!(tuples, 66);
    assert_eq!(
        statistics
            .usage()
            .get(laneflow_static_contract::CanonicalNetworkDimension::CanonicalRelationCount),
        tuples
            + roles.get(SourceRelationRole::SignalPhaseState)
            + roles.get(SourceRelationRole::CanonicalFrameLaneEdgeGeometry)
            + roles.get(SourceRelationRole::CanonicalFrameFacilityBandGeometry)
            + roles.get(SourceRelationRole::CanonicalFrameConflictZoneRegion)
    );
}

#[test]
fn lfca_statistics_reject_a_missing_entity_section() {
    let candidate = full_spatial_portable_fixture_candidate();
    let lfsm = laneflow_format::preflight_object_values(
        candidate.source_map().bytes(),
        laneflow_static_contract::PortableObjectKind::SourceMap,
        laneflow_format::FormatLimits::HARD,
    )
    .unwrap()
    .registry_view();
    assert_eq!(
        lfca_statistics(lfsm, PortableEmissionError::DiffBaseSemanticMismatch),
        Err(PortableEmissionError::DiffBaseSemanticMismatch)
    );
}

#[test]
fn lfca_projection_registry_matches_canonical_relation_capacity() {
    for role in SourceRelationRole::ALL {
        let counted = matches!(
            role.canonical_relation_capacity(),
            laneflow_static_contract::CanonicalRelationCapacity::Counted
        );
        assert_eq!(lfca_projection(role).is_some(), counted, "{role:?}");
    }
}
