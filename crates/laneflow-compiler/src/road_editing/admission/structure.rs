//! 在分配前按 lowering 的拥有型存储形状推导结构 backing；不含字符串和位置 context。
//!
//! 外层 enum 已包含 inline 字段、Box/Arc handle 和引用。这里只追加独立 slice/Box
//! 的元素，不再把每个 wire table、引用和关系都当作一个最大的声明 enum。

use laneflow_static_contract::{
    LaneEdgeKind, ParticipantClassKind, ParticipantStreamKind, SignalGroupKind,
};

use super::*;
use crate::SourceLocation;
use crate::declaration::{
    AuthoringLaneDeclaration, AuthoringPoint2F64, ConflictPassageDeclaration,
    ConflictZoneRegionDeclaration, OwnedCorridorElementReference, OwnedEntityReference,
    ParkingLaneAnchorDeclaration, PolicyEvidenceDeclaration, PolicyGapProfileDeclaration,
    PolicyGateRuleDeclaration, PolicyStreamRuleDeclaration, SignalGroupStateDeclaration,
    SignalPhaseDeclaration,
};

#[derive(Clone, Copy)]
pub(super) struct StructureSizing {
    pub(super) retained_bytes: u64,
    pub(super) authoring_curve_bytes: u64,
}

fn bytes<T>(count: usize) -> u64 {
    size_bytes::<T>(u64::try_from(count).unwrap_or(u64::MAX))
}

impl StructureSizing {
    pub(super) fn from_root(
        root: wire::RoadEditingSource<'_>,
        counts: RoadEditingPreflightCounts,
    ) -> Self {
        let top_level_count = counts
            .declaration_count()
            .saturating_sub(u64::try_from(root.authoring_lanes().len()).unwrap_or(u64::MAX))
            .saturating_sub(u64::try_from(root.signal_phases().len()).unwrap_or(u64::MAX));
        let mut retained = size_bytes::<TypedAstDeclaration>(top_level_count)
            .saturating_add(bytes::<AuthoringLaneDeclaration>(
                root.authoring_lanes().len(),
            ))
            // 每条 authoring lane 恰有一条派生 edge；lane 的 inline header 已计入上项。
            .saturating_add(bytes::<OwnedEntityReference<LaneEdgeKind>>(
                root.authoring_lanes().len(),
            ))
            .saturating_add(bytes::<SignalPhaseDeclaration>(root.signal_phases().len()))
            .saturating_add(bytes::<ConflictZoneRegionDeclaration>(
                root.conflict_zone_regions().len(),
            ))
            .saturating_add(size_bytes::<AuthoringPoint2F64>(
                counts.conflict_region_point_count(),
            ));
        for corridor in root.road_corridors() {
            retained = retained.saturating_add(bytes::<OwnedCorridorElementReference>(
                corridor.elements().len(),
            ));
        }
        let mut authoring_curve_bytes = alignment_input_scratch_bytes_from_root(root);
        for edge in root.lane_edges() {
            retained = retained.saturating_add(bytes::<OwnedEntityReference<LaneEdgeKind>>(
                edge.successors().len(),
            ));
            if let Some(curve) = edge.explicit_geometry() {
                authoring_curve_bytes =
                    authoring_curve_bytes.saturating_add(curve_backing_bytes(curve));
            }
        }
        for junction in root.junctions() {
            retained = retained
                .saturating_add(bytes::<OwnedEntityReference<LaneEdgeKind>>(
                    junction.approach_edges().len(),
                ))
                .saturating_add(bytes::<OwnedEntityReference<LaneEdgeKind>>(
                    junction.internal_edges().len(),
                ));
        }
        for path in root.maneuver_paths() {
            retained = retained.saturating_add(bytes::<OwnedEntityReference<LaneEdgeKind>>(
                path.internal_edges().len(),
            ));
        }
        for phase in root.signal_phases() {
            retained =
                retained.saturating_add(bytes::<SignalGroupStateDeclaration>(phase.states().len()));
        }
        for controller in root.signal_controllers() {
            retained = retained.saturating_add(bytes::<OwnedEntityReference<SignalGroupKind>>(
                controller.signal_groups().len(),
            ));
        }
        for facility in root.parking_facilities() {
            let anchors = facility
                .virtual_entries()
                .len()
                .saturating_add(facility.virtual_exits().len());
            retained = retained
                .saturating_add(bytes::<ParkingLaneAnchorDeclaration>(anchors))
                .saturating_add(bytes::<SourceLocation>(anchors));
        }
        // ParkingSpace 的两个 anchor 本体内嵌在 enum 中，但 progress_span 独立装箱。
        retained = retained.saturating_add(bytes::<SourceLocation>(
            root.parking_spaces().len().saturating_mul(2),
        ));
        for rule in root.access_rules() {
            retained =
                retained.saturating_add(bytes::<OwnedEntityReference<ParticipantClassKind>>(
                    rule.participant_classes().len(),
                ));
        }
        for stream in root.participant_streams() {
            retained = retained
                .saturating_add(bytes::<ConflictPassageDeclaration>(stream.passages().len()));
        }
        for policy in root.right_of_way_policy_sets() {
            retained = retained.saturating_add(policy_backing_bytes(policy));
        }
        Self {
            retained_bytes: retained,
            authoring_curve_bytes,
        }
    }
}

fn curve_backing_bytes(curve: wire::CurveProgram<'_>) -> u64 {
    bytes::<AuthoringCurveSegmentDeclaration>(curve.segments().len()).saturating_add(bytes::<
        AuthoringCurveCubicPointSpans,
    >(
        curve
            .segments()
            .iter()
            .filter(|segment| {
                segment.geometry_type() == wire::CurveSegmentGeometry::CubicBezierSegment
            })
            .count(),
    ))
}

fn policy_backing_bytes(policy: wire::RightOfWayPolicySet<'_>) -> u64 {
    let mut total = bytes::<PolicyEvidenceDeclaration>(policy.evidence().len())
        .saturating_add(bytes::<PolicyGapProfileDeclaration>(
            policy.gap_profiles().len(),
        ))
        .saturating_add(bytes::<PolicyStreamRuleDeclaration>(
            policy.stream_rules().len(),
        ))
        .saturating_add(bytes::<PolicyGateRuleDeclaration>(
            policy.gate_rules().len(),
        ))
        .saturating_add(bytes::<SourceLocation>(
            3 + usize::from(policy.regulation().source().is_some()),
        ));
    for evidence in policy.evidence() {
        total = total.saturating_add(bytes::<SourceLocation>(
            2 + usize::from(evidence.description().is_some()),
        ));
    }
    total = total.saturating_add(bytes::<SourceLocation>(
        policy.gap_profiles().len().saturating_mul(5),
    ));
    for rule in policy.stream_rules() {
        total = total
            .saturating_add(bytes::<SourceLocation>(
                5 + usize::from(rule.participant_classes().is_some())
                    + usize::from(rule.gap_profile_key().is_some()),
            ))
            .saturating_add(bytes::<OwnedEntityReference<ParticipantClassKind>>(
                rule.participant_classes().map_or(0, |values| values.len()),
            ))
            .saturating_add(bytes::<OwnedEntityReference<ParticipantStreamKind>>(
                rule.yield_to_streams().len(),
            ))
            .saturating_add(bytes::<Arc<str>>(rule.evidence_keys().len()));
    }
    for rule in policy.gate_rules() {
        total = total
            .saturating_add(bytes::<SourceLocation>(
                5 + usize::from(rule.participant_classes().is_some()),
            ))
            .saturating_add(bytes::<OwnedEntityReference<ParticipantClassKind>>(
                rule.participant_classes().map_or(0, |values| values.len()),
            ))
            .saturating_add(bytes::<Arc<str>>(rule.evidence_keys().len()));
    }
    total
}

// 开发构建从已分配树独立核对 wire 预收费，防止 lowering 新增 backing 后漏更新预算。
#[cfg(debug_assertions)]
pub(super) fn assert_matches_lowered(
    sizing: StructureSizing,
    declarations: &[TypedAstDeclaration],
    alignments: &[RoadAlignmentDeclaration],
    regions: &[ConflictZoneRegionDeclaration],
) {
    use crate::declaration::{AuthoringCurveProgramDeclaration, LaneEdgeGeometryAuthority};

    fn storage<T>(values: &[T]) -> u64 {
        u64::try_from(core::mem::size_of_val(values)).unwrap_or(u64::MAX)
    }
    fn curve_storage(curve: &AuthoringCurveProgramDeclaration) -> u64 {
        curve
            .segments
            .iter()
            .fold(storage(&curve.segments), |total, segment| {
                total.saturating_add(match &segment.point_spans {
                    AuthoringCurveSegmentPointSpans::Line { .. } => 0,
                    AuthoringCurveSegmentPointSpans::CubicBezier(spans) => {
                        u64::try_from(core::mem::size_of_val(spans.as_ref())).unwrap_or(u64::MAX)
                    }
                })
            })
    }

    let mut retained = storage(declarations).saturating_add(storage(regions));
    for region in regions {
        retained = retained.saturating_add(storage(&region.ring_xz));
    }
    let mut authoring = storage(alignments);
    for alignment in alignments {
        authoring = authoring.saturating_add(curve_storage(&alignment.reference_line));
    }
    for declaration in declarations {
        retained = retained.saturating_add(match declaration {
            TypedAstDeclaration::LaneEdge(edge) => {
                if let LaneEdgeGeometryAuthority::Authoring {
                    explicit_curve: Some(curve),
                } = &edge.geometry_authority
                {
                    authoring = authoring.saturating_add(curve_storage(curve));
                }
                storage(&edge.successors)
            }
            TypedAstDeclaration::RoadCorridor(corridor) => storage(&corridor.elements),
            TypedAstDeclaration::RoadSection(section) => section
                .lanes
                .iter()
                .fold(storage(&section.lanes), |total, lane| {
                    total.saturating_add(storage(&lane.edge_chain))
                }),
            TypedAstDeclaration::Junction(junction) => {
                storage(&junction.approach_edges).saturating_add(storage(&junction.internal_edges))
            }
            TypedAstDeclaration::ManeuverPath(path) => storage(&path.internal_edges),
            TypedAstDeclaration::SignalController(controller) => controller.phases.iter().fold(
                storage(&controller.phases).saturating_add(storage(&controller.signal_groups)),
                |total, phase| total.saturating_add(storage(&phase.states)),
            ),
            TypedAstDeclaration::ParkingFacility(facility) => {
                let anchors = facility
                    .virtual_entries
                    .iter()
                    .chain(&facility.virtual_exits);
                anchors.fold(
                    storage(&facility.virtual_entries)
                        .saturating_add(storage(&facility.virtual_exits)),
                    |total, anchor| {
                        total.saturating_add(storage(core::slice::from_ref(
                            anchor.progress_span.as_ref(),
                        )))
                    },
                )
            }
            TypedAstDeclaration::ParkingSpace(space) => {
                storage(core::slice::from_ref(space.entry.progress_span.as_ref())).saturating_add(
                    storage(core::slice::from_ref(space.exit.progress_span.as_ref())),
                )
            }
            TypedAstDeclaration::CanonicalFrame(frame) => {
                assert!(
                    frame.lane_edge_geometries.is_empty(),
                    "RoadEditing canonical geometry is emitted after lowering"
                );
                0
            }
            TypedAstDeclaration::AccessRule(rule) => storage(&rule.participant_classes),
            TypedAstDeclaration::ParticipantStream(stream) => storage(&stream.passages),
            TypedAstDeclaration::RightOfWayPolicySet(policy) => {
                let mut total = storage(&policy.evidence)
                    .saturating_add(storage(&policy.gap_profiles))
                    .saturating_add(storage(&policy.stream_rules))
                    .saturating_add(storage(&policy.gate_rules))
                    .saturating_add(storage(&policy.contributing));
                for evidence in &policy.evidence {
                    total = total.saturating_add(storage(&evidence.source.contributing));
                }
                for gap in &policy.gap_profiles {
                    total = total.saturating_add(storage(&gap.source.contributing));
                }
                for rule in &policy.stream_rules {
                    total = total
                        .saturating_add(storage(&rule.source.contributing))
                        .saturating_add(storage(rule.classes.as_deref().unwrap_or(&[])))
                        .saturating_add(storage(&rule.yield_to))
                        .saturating_add(storage(&rule.evidence));
                }
                for rule in &policy.gate_rules {
                    total = total
                        .saturating_add(storage(&rule.source.contributing))
                        .saturating_add(storage(rule.classes.as_deref().unwrap_or(&[])))
                        .saturating_add(storage(&rule.evidence));
                }
                total
            }
            TypedAstDeclaration::LaneGroup(_)
            | TypedAstDeclaration::FacilityBand(_)
            | TypedAstDeclaration::Movement(_)
            | TypedAstDeclaration::StopLine(_)
            | TypedAstDeclaration::ManeuverGate(_)
            | TypedAstDeclaration::SignalGroup(_)
            | TypedAstDeclaration::ParticipantClass(_)
            | TypedAstDeclaration::VehicleProfile(_)
            | TypedAstDeclaration::WaitingZone(_)
            | TypedAstDeclaration::ConflictZone(_) => 0,
        });
    }
    assert_eq!(
        sizing.retained_bytes, retained,
        "wire-derived retained structure must match lowered backing"
    );
    assert_eq!(
        sizing.authoring_curve_bytes, authoring,
        "wire-derived temporary curves must match lowered backing"
    );
}
