use std::sync::Arc;

use laneflow_road_editing_wire::generated::lane_flow::road_editing::v1 as wire;
use laneflow_static_contract::EntityKind;

use crate::{
    RoadEditingAddressKind, RoadEditingDocumentIdentity, RoadEditingLocationContext,
    RoadEditingOwner, RoadEditingPropertyPath, RoadEditingPropertyStep, RoadEditingRelationKind,
    RoadEditingRelationOccurrence, RoadEditingSourceAddress, RoadEditingSourceLocation,
    RoadEditingStructKind, RoadEditingSubject, RoadEditingTableKind, RoadEditingUnionKind,
    SourceLocation,
};

/// verifier 后的第二遍只收集来源位置需要保留的唯一 token；不复制完整 wire 字符串集。
pub(crate) struct RoadEditingLocationFactory {
    context: Arc<RoadEditingLocationContext>,
    document_identity: RoadEditingDocumentIdentity,
}

impl RoadEditingLocationFactory {
    pub(crate) fn input_module_header(expected_source_document_key: &str) -> SourceLocation {
        Self {
            context: empty_context(),
            document_identity: RoadEditingDocumentIdentity::input(Arc::from(
                expected_source_document_key,
            )),
        }
        .module_header()
    }

    pub(crate) fn verified_module_header(
        module_namespace: &str,
        source_document_key: &str,
    ) -> SourceLocation {
        Self {
            context: empty_context(),
            document_identity: RoadEditingDocumentIdentity::verified(
                Arc::from(module_namespace),
                Arc::from(source_document_key),
            ),
        }
        .module_header()
    }

    pub(crate) fn from_verified_root(root: wire::RoadEditingSource<'_>) -> Self {
        let header = root.module_header();
        let mut strings = vec![Arc::<str>::from(header.authoring_namespace_id())];
        let mut canvas_selections = Vec::<Arc<str>>::new();

        macro_rules! collect_root {
            ($values:expr, $key:ident) => {
                for value in $values {
                    strings.push(Arc::from(value.$key()));
                    collect_canvas(&mut canvas_selections, value.canvas_selection());
                }
            };
        }

        for alignment in root.road_alignments() {
            strings.push(Arc::from(alignment.road_alignment_key()));
            collect_canvas(&mut canvas_selections, alignment.canvas_selection());
            collect_curve_canvas(&mut canvas_selections, alignment.reference_line());
        }
        collect_root!(root.road_corridors(), road_corridor_key);
        collect_root!(root.road_sections(), road_section_key);
        collect_root!(root.authoring_lanes(), authoring_lane_key);
        for edge in root.lane_edges() {
            strings.push(Arc::from(edge.lane_edge_key()));
            collect_canvas(&mut canvas_selections, edge.canvas_selection());
            if let Some(curve) = edge.explicit_geometry() {
                collect_curve_canvas(&mut canvas_selections, curve);
            }
        }
        collect_root!(root.junctions(), junction_key);
        collect_root!(root.movements(), movement_key);
        collect_root!(root.maneuver_paths(), maneuver_path_key);
        collect_root!(root.maneuver_gates(), maneuver_gate_key);
        collect_root!(root.waiting_zones(), waiting_zone_key);
        collect_root!(root.stop_lines(), stop_line_key);
        collect_root!(root.signal_groups(), signal_group_key);
        collect_root!(root.signal_controllers(), signal_controller_key);
        collect_root!(root.signal_phases(), signal_phase_key);
        collect_root!(root.parking_areas(), parking_area_key);
        collect_root!(root.parking_spaces(), parking_space_key);
        collect_root!(root.lane_groups(), lane_group_key);
        collect_root!(root.facility_bands(), facility_band_key);
        collect_root!(root.participant_classes(), participant_class_key);
        collect_root!(root.access_rules(), access_rule_key);
        collect_root!(root.vehicle_profiles(), vehicle_profile_key);
        collect_root!(root.static_routes(), static_route_key);
        collect_root!(root.canonical_frames(), canonical_frame_key);

        strings.sort_unstable_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
        strings.dedup_by(|left, right| left.as_bytes() == right.as_bytes());
        canvas_selections.sort_unstable_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
        canvas_selections.dedup_by(|left, right| left.as_bytes() == right.as_bytes());
        let mut property_paths = closed_property_paths();
        property_paths.sort_unstable();
        property_paths.dedup();

        let namespace: Arc<str> = Arc::from(header.authoring_namespace_id());
        let document_key: Arc<str> = Arc::from(header.source_document_key());
        Self {
            context: Arc::new(RoadEditingLocationContext::new(
                strings.into_boxed_slice(),
                property_paths.into_boxed_slice(),
                canvas_selections.into_boxed_slice(),
            )),
            document_identity: RoadEditingDocumentIdentity::verified(namespace, document_key),
        }
    }

    pub(crate) fn declaration(
        &self,
        entity_kind: EntityKind,
        owner_local_keys: &[&str],
        local_key: &str,
        canvas_selection: Option<&str>,
    ) -> SourceLocation {
        self.location(
            RoadEditingSubject::Declaration {
                address: self.address(
                    RoadEditingAddressKind::Declaration(entity_kind),
                    owner_local_keys,
                    local_key,
                ),
            },
            None,
            canvas_selection,
        )
    }

    pub(crate) fn road_alignment(
        &self,
        road_alignment_key: &str,
        canvas_selection: Option<&str>,
    ) -> SourceLocation {
        self.location(
            RoadEditingSubject::RoadAlignment {
                address: self.address(
                    RoadEditingAddressKind::RoadAlignment,
                    &[],
                    road_alignment_key,
                ),
            },
            None,
            canvas_selection,
        )
    }

    pub(crate) fn road_alignment_property(
        &self,
        road_alignment_key: &str,
        steps: &[RoadEditingPropertyStep],
        canvas_selection: Option<&str>,
    ) -> SourceLocation {
        self.location(
            RoadEditingSubject::RoadAlignment {
                address: self.address(
                    RoadEditingAddressKind::RoadAlignment,
                    &[],
                    road_alignment_key,
                ),
            },
            Some(steps),
            canvas_selection,
        )
    }

    pub(crate) fn road_alignment_owner_local(
        &self,
        road_alignment_key: &str,
        relation: RoadEditingRelationKind,
        occurrence: RoadEditingRelationOccurrence,
        steps: &[RoadEditingPropertyStep],
        canvas_selection: Option<&str>,
    ) -> SourceLocation {
        self.owner_local_address(
            RoadEditingAddressKind::RoadAlignment,
            &[],
            road_alignment_key,
            relation,
            occurrence,
            steps,
            canvas_selection,
        )
    }

    pub(crate) fn property(
        &self,
        entity_kind: EntityKind,
        owner_local_keys: &[&str],
        local_key: &str,
        steps: &[RoadEditingPropertyStep],
        canvas_selection: Option<&str>,
    ) -> SourceLocation {
        self.location(
            RoadEditingSubject::Declaration {
                address: self.address(
                    RoadEditingAddressKind::Declaration(entity_kind),
                    owner_local_keys,
                    local_key,
                ),
            },
            Some(steps),
            canvas_selection,
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "closed typed location fields remain explicit at relation call sites"
    )]
    pub(crate) fn owner_local(
        &self,
        owner_kind: EntityKind,
        owner_local_keys: &[&str],
        owner_key: &str,
        relation: RoadEditingRelationKind,
        occurrence: RoadEditingRelationOccurrence,
        steps: &[RoadEditingPropertyStep],
        canvas_selection: Option<&str>,
    ) -> SourceLocation {
        self.owner_local_address(
            RoadEditingAddressKind::Declaration(owner_kind),
            owner_local_keys,
            owner_key,
            relation,
            occurrence,
            steps,
            canvas_selection,
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "closed typed location fields remain explicit at relation call sites"
    )]
    fn owner_local_address(
        &self,
        owner_kind: RoadEditingAddressKind,
        owner_local_keys: &[&str],
        owner_key: &str,
        relation: RoadEditingRelationKind,
        occurrence: RoadEditingRelationOccurrence,
        steps: &[RoadEditingPropertyStep],
        canvas_selection: Option<&str>,
    ) -> SourceLocation {
        self.location(
            RoadEditingSubject::OwnerLocal {
                owner: RoadEditingOwner::Address(self.address(
                    owner_kind,
                    owner_local_keys,
                    owner_key,
                )),
                relation,
                occurrence,
            },
            Some(steps),
            canvas_selection,
        )
    }

    pub(crate) fn module_owner_local(
        &self,
        relation: RoadEditingRelationKind,
        occurrence: RoadEditingRelationOccurrence,
        steps: &[RoadEditingPropertyStep],
    ) -> SourceLocation {
        self.location(
            RoadEditingSubject::OwnerLocal {
                owner: RoadEditingOwner::ModuleHeader,
                relation,
                occurrence,
            },
            Some(steps),
            None,
        )
    }

    pub(crate) fn module_header(&self) -> SourceLocation {
        self.location(RoadEditingSubject::ModuleHeader, None, None)
    }

    fn address(
        &self,
        kind: RoadEditingAddressKind,
        owner_local_keys: &[&str],
        local_key: &str,
    ) -> RoadEditingSourceAddress {
        let module_namespace = self
            .document_identity
            .module_namespace()
            .expect("verified road-editing identity retains namespace");
        RoadEditingSourceAddress::new(
            self.context.string_ordinal_for(module_namespace),
            kind,
            owner_local_keys
                .iter()
                .map(|key| self.context.string_ordinal_for(key)),
            self.context.string_ordinal_for(local_key),
        )
    }

    fn location(
        &self,
        subject: RoadEditingSubject,
        property_steps: Option<&[RoadEditingPropertyStep]>,
        canvas_selection: Option<&str>,
    ) -> SourceLocation {
        let property_path = property_steps.map(|steps| {
            let path = RoadEditingPropertyPath::new(steps.to_vec().into_boxed_slice());
            self.context.property_path_ordinal_for(&path)
        });
        let canvas_selection =
            canvas_selection.map(|value| self.context.canvas_selection_ordinal_for(value));
        SourceLocation::RoadEditing(RoadEditingSourceLocation::new(
            Arc::clone(&self.context),
            self.document_identity.clone(),
            subject,
            property_path,
            canvas_selection,
            None,
        ))
    }
}

fn empty_context() -> Arc<RoadEditingLocationContext> {
    Arc::new(RoadEditingLocationContext::new(
        Box::default(),
        Box::default(),
        Box::default(),
    ))
}

fn collect_canvas(output: &mut Vec<Arc<str>>, value: Option<&str>) {
    if let Some(value) = value {
        output.push(Arc::from(value));
    }
}

fn collect_curve_canvas(output: &mut Vec<Arc<str>>, curve: wire::CurveProgram<'_>) {
    for segment in curve.segments() {
        collect_canvas(output, segment.canvas_selection());
    }
}

fn closed_property_paths() -> Vec<RoadEditingPropertyPath> {
    let tables = [
        (RoadEditingTableKind::RoadEditingSource, 26_u16),
        (RoadEditingTableKind::ModuleHeader, 3),
        (RoadEditingTableKind::Provenance, 5),
        (RoadEditingTableKind::LineSegment, 0),
        (RoadEditingTableKind::CubicBezierSegment, 2),
        (RoadEditingTableKind::CurveSegment, 2),
        (RoadEditingTableKind::CurveProgram, 1),
        (RoadEditingTableKind::RoadAlignment, 3),
        (RoadEditingTableKind::CorridorElement, 1),
        (RoadEditingTableKind::RoadCorridor, 8),
        (RoadEditingTableKind::RoadSection, 4),
        (RoadEditingTableKind::AuthoringLane, 6),
        (RoadEditingTableKind::LaneEdge, 4),
        (RoadEditingTableKind::Junction, 3),
        (RoadEditingTableKind::Movement, 4),
        (RoadEditingTableKind::ManeuverPath, 5),
        (RoadEditingTableKind::ManeuverGate, 6),
        (RoadEditingTableKind::WaitingZone, 5),
        (RoadEditingTableKind::StopLine, 2),
        (RoadEditingTableKind::SignalGroup, 1),
        (RoadEditingTableKind::SignalController, 4),
        (RoadEditingTableKind::SignalPhaseState, 1),
        (RoadEditingTableKind::SignalPhase, 4),
        (RoadEditingTableKind::ParkingArea, 1),
        (RoadEditingTableKind::ParkingLaneAnchor, 1),
        (RoadEditingTableKind::ParkingSpaceGeometry, 3),
        (RoadEditingTableKind::ParkingSpace, 5),
        (RoadEditingTableKind::LaneGroup, 2),
        (RoadEditingTableKind::FacilityBand, 4),
        (RoadEditingTableKind::ParticipantClass, 2),
        (RoadEditingTableKind::AccessRegulation, 2),
        (RoadEditingTableKind::AccessRule, 7),
        (RoadEditingTableKind::IidmVehicleProfile, 6),
        (RoadEditingTableKind::VehicleProfile, 3),
        (RoadEditingTableKind::StaticRoute, 2),
        (RoadEditingTableKind::CanonicalFrame, 1),
    ];
    let mut paths = Vec::new();
    for (table, last_field_id) in tables {
        for field_id in 0..=last_field_id {
            paths.push(RoadEditingPropertyPath::new(Box::new([
                RoadEditingPropertyStep::TableField { table, field_id },
            ])));
        }
    }
    for (table, field_id, structure, members) in [
        (
            RoadEditingTableKind::Provenance,
            2,
            RoadEditingStructKind::Digest256,
            1_u8,
        ),
        (
            RoadEditingTableKind::Provenance,
            3,
            RoadEditingStructKind::Digest256,
            1,
        ),
        (
            RoadEditingTableKind::Provenance,
            4,
            RoadEditingStructKind::OptionalU64,
            1,
        ),
        (
            RoadEditingTableKind::CurveProgram,
            0,
            RoadEditingStructKind::Vec3F64,
            3,
        ),
        (
            RoadEditingTableKind::LineSegment,
            0,
            RoadEditingStructKind::Vec3F64,
            3,
        ),
        (
            RoadEditingTableKind::AuthoringLane,
            3,
            RoadEditingStructKind::LinearWidthProfile,
            2,
        ),
        (
            RoadEditingTableKind::FacilityBand,
            2,
            RoadEditingStructKind::LinearWidthProfile,
            2,
        ),
    ] {
        for member_id in 0..members {
            paths.push(RoadEditingPropertyPath::new(Box::new([
                RoadEditingPropertyStep::TableField { table, field_id },
                RoadEditingPropertyStep::StructMember {
                    structure,
                    member_id,
                },
            ])));
        }
    }
    for (outer_table, outer_field_id, inner_table, inner_last_field_id) in [
        (
            RoadEditingTableKind::ModuleHeader,
            3,
            RoadEditingTableKind::Provenance,
            5_u16,
        ),
        (
            RoadEditingTableKind::RoadAlignment,
            2,
            RoadEditingTableKind::CurveProgram,
            1,
        ),
        (
            RoadEditingTableKind::LaneEdge,
            3,
            RoadEditingTableKind::CurveProgram,
            1,
        ),
        (
            RoadEditingTableKind::RoadCorridor,
            7,
            RoadEditingTableKind::CorridorElement,
            1,
        ),
        (
            RoadEditingTableKind::SignalPhase,
            2,
            RoadEditingTableKind::SignalPhaseState,
            1,
        ),
        (
            RoadEditingTableKind::ParkingSpace,
            2,
            RoadEditingTableKind::ParkingLaneAnchor,
            1,
        ),
        (
            RoadEditingTableKind::ParkingSpace,
            3,
            RoadEditingTableKind::ParkingLaneAnchor,
            1,
        ),
        (
            RoadEditingTableKind::ParkingSpace,
            4,
            RoadEditingTableKind::ParkingSpaceGeometry,
            3,
        ),
        (
            RoadEditingTableKind::AccessRule,
            5,
            RoadEditingTableKind::AccessRegulation,
            2,
        ),
        (
            RoadEditingTableKind::VehicleProfile,
            2,
            RoadEditingTableKind::IidmVehicleProfile,
            6,
        ),
    ] {
        for inner_field_id in 0..=inner_last_field_id {
            paths.push(RoadEditingPropertyPath::new(Box::new([
                RoadEditingPropertyStep::TableField {
                    table: outer_table,
                    field_id: outer_field_id,
                },
                RoadEditingPropertyStep::TableField {
                    table: inner_table,
                    field_id: inner_field_id,
                },
            ])));
        }
    }
    for (outer_table, outer_field_id) in [
        (RoadEditingTableKind::RoadAlignment, 2_u16),
        (RoadEditingTableKind::LaneEdge, 3),
    ] {
        for member_id in 0..3_u8 {
            paths.push(RoadEditingPropertyPath::new(Box::new([
                RoadEditingPropertyStep::TableField {
                    table: outer_table,
                    field_id: outer_field_id,
                },
                RoadEditingPropertyStep::TableField {
                    table: RoadEditingTableKind::CurveProgram,
                    field_id: 0,
                },
                RoadEditingPropertyStep::StructMember {
                    structure: RoadEditingStructKind::Vec3F64,
                    member_id,
                },
            ])));
        }
    }
    for (field_id, structure, members) in [
        (2_u16, RoadEditingStructKind::Digest256, 1_u8),
        (3, RoadEditingStructKind::Digest256, 1),
        (4, RoadEditingStructKind::OptionalU64, 1),
    ] {
        for member_id in 0..members {
            paths.push(RoadEditingPropertyPath::new(Box::new([
                RoadEditingPropertyStep::TableField {
                    table: RoadEditingTableKind::ModuleHeader,
                    field_id: 3,
                },
                RoadEditingPropertyStep::TableField {
                    table: RoadEditingTableKind::Provenance,
                    field_id,
                },
                RoadEditingPropertyStep::StructMember {
                    structure,
                    member_id,
                },
            ])));
        }
    }
    for (field_id, members) in [(0_u16, 3_u8), (1, 3), (2, 3)] {
        for member_id in 0..members {
            paths.push(RoadEditingPropertyPath::new(Box::new([
                RoadEditingPropertyStep::TableField {
                    table: RoadEditingTableKind::CubicBezierSegment,
                    field_id,
                },
                RoadEditingPropertyStep::StructMember {
                    structure: RoadEditingStructKind::Vec3F64,
                    member_id,
                },
            ])));
        }
    }
    for (outer_table, outer_field_id, inner_table, inner_field_id) in [
        (
            RoadEditingTableKind::SignalPhase,
            2,
            RoadEditingTableKind::SignalPhaseState,
            0,
        ),
        (
            RoadEditingTableKind::ParkingSpace,
            2,
            RoadEditingTableKind::ParkingLaneAnchor,
            0,
        ),
        (
            RoadEditingTableKind::ParkingSpace,
            3,
            RoadEditingTableKind::ParkingLaneAnchor,
            0,
        ),
    ] {
        paths.push(RoadEditingPropertyPath::new(Box::new([
            RoadEditingPropertyStep::TableField {
                table: outer_table,
                field_id: outer_field_id,
            },
            RoadEditingPropertyStep::TableField {
                table: inner_table,
                field_id: inner_field_id,
            },
        ])));
    }
    for (variant, table, field_count) in [
        (1_u8, RoadEditingTableKind::LineSegment, 1_u16),
        (2, RoadEditingTableKind::CubicBezierSegment, 3),
    ] {
        for field_id in 0..field_count {
            for member_id in 0..3_u8 {
                paths.push(RoadEditingPropertyPath::new(Box::new([
                    RoadEditingPropertyStep::TableField {
                        table: RoadEditingTableKind::CurveSegment,
                        field_id: 1,
                    },
                    RoadEditingPropertyStep::UnionVariant {
                        union: RoadEditingUnionKind::CurveSegmentGeometry,
                        discriminant: variant,
                    },
                    RoadEditingPropertyStep::TableField { table, field_id },
                    RoadEditingPropertyStep::StructMember {
                        structure: RoadEditingStructKind::Vec3F64,
                        member_id,
                    },
                ])));
            }
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::road_editing::{
        CanonicalFrameInput, RoadEditingDeclaration, RoadEditingModuleHeader,
        RoadEditingModuleInput, RoadEditingProvenance, RoadEditingSourceModuleBuilder,
        RoadEditingSourceWriter,
    };
    use crate::{CompileLimits, GeometryAccuracyProfile, GeometryDirectionProfile};

    #[test]
    fn factory_resolves_owner_address_property_and_canvas_after_wire_order_changes() {
        let limits = CompileLimits::p100_initial_v2();
        let header = RoadEditingModuleHeader::try_new(
            "city/main",
            "roads/main",
            Vec::new(),
            RoadEditingProvenance::direct("test").unwrap(),
        )
        .unwrap();
        let mut builder = RoadEditingSourceModuleBuilder::new(
            header,
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            &limits,
        )
        .unwrap();
        builder
            .add_declaration(RoadEditingDeclaration::CanonicalFrame(
                CanonicalFrameInput::try_new("frame-main")
                    .unwrap()
                    .with_canvas_selection("canvas/frame-main")
                    .unwrap(),
            ))
            .unwrap();
        let module = builder.finish().unwrap();
        let bytes = RoadEditingSourceWriter::new(&limits).write(module).unwrap();
        let input = RoadEditingModuleInput::try_new("roads/main", bytes.as_bytes(), None).unwrap();
        let verified = super::super::reader::verify_source(input, &limits, 0, 0).unwrap();
        let factory = RoadEditingLocationFactory::from_verified_root(verified.root());
        let location = factory.property(
            EntityKind::CanonicalFrame,
            &[],
            "frame-main",
            &[RoadEditingPropertyStep::TableField {
                table: RoadEditingTableKind::CanonicalFrame,
                field_id: 0,
            }],
            Some("canvas/frame-main"),
        );
        let road = location.road_editing().unwrap();
        let RoadEditingSubject::Declaration { address } = road.subject() else {
            panic!("expected declaration subject");
        };
        assert_eq!(address.module_namespace(road.context()), "city/main");
        assert_eq!(address.local_key(road.context()), "frame-main");
        assert_eq!(road.canvas_selection(), Some("canvas/frame-main"));
        assert_eq!(road.property_path().unwrap().steps().len(), 1);
    }

    #[test]
    fn factory_represents_module_import_as_module_owned_relation() {
        let limits = CompileLimits::p100_initial_v2();
        let header = RoadEditingModuleHeader::try_new(
            "city/main",
            "roads/main",
            vec!["city/base".to_owned()],
            RoadEditingProvenance::direct("test").unwrap(),
        )
        .unwrap();
        let module = RoadEditingSourceModuleBuilder::new(
            header,
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            &limits,
        )
        .unwrap()
        .finish()
        .unwrap();
        let bytes = RoadEditingSourceWriter::new(&limits).write(module).unwrap();
        let input = RoadEditingModuleInput::try_new("roads/main", bytes.as_bytes(), None).unwrap();
        let verified = super::super::reader::verify_source(input, &limits, 0, 0).unwrap();
        let factory = RoadEditingLocationFactory::from_verified_root(verified.root());
        let location = factory.module_owner_local(
            RoadEditingRelationKind::Import,
            RoadEditingRelationOccurrence::CanonicalSetOrdinal(0),
            &[RoadEditingPropertyStep::TableField {
                table: RoadEditingTableKind::ModuleHeader,
                field_id: 2,
            }],
        );
        let road = location.road_editing().unwrap();

        assert!(matches!(
            road.subject(),
            RoadEditingSubject::OwnerLocal {
                owner: RoadEditingOwner::ModuleHeader,
                relation: RoadEditingRelationKind::Import,
                occurrence: RoadEditingRelationOccurrence::CanonicalSetOrdinal(0),
            }
        ));
    }

    #[test]
    fn closed_paths_cover_nested_table_leaves() {
        let paths = closed_property_paths();
        for expected in [
            RoadEditingPropertyPath::new(Box::new([
                RoadEditingPropertyStep::TableField {
                    table: RoadEditingTableKind::ParkingSpace,
                    field_id: 2,
                },
                RoadEditingPropertyStep::TableField {
                    table: RoadEditingTableKind::ParkingLaneAnchor,
                    field_id: 1,
                },
            ])),
            RoadEditingPropertyPath::new(Box::new([
                RoadEditingPropertyStep::TableField {
                    table: RoadEditingTableKind::VehicleProfile,
                    field_id: 2,
                },
                RoadEditingPropertyStep::TableField {
                    table: RoadEditingTableKind::IidmVehicleProfile,
                    field_id: 6,
                },
            ])),
            RoadEditingPropertyPath::new(Box::new([
                RoadEditingPropertyStep::TableField {
                    table: RoadEditingTableKind::ModuleHeader,
                    field_id: 3,
                },
                RoadEditingPropertyStep::TableField {
                    table: RoadEditingTableKind::Provenance,
                    field_id: 2,
                },
                RoadEditingPropertyStep::StructMember {
                    structure: RoadEditingStructKind::Digest256,
                    member_id: 0,
                },
            ])),
        ] {
            assert!(
                paths.contains(&expected),
                "missing nested path: {expected:?}"
            );
        }
    }

    #[test]
    fn road_alignment_segment_location_keeps_address_and_canvas() {
        let path = RoadEditingPropertyPath::new(Box::new([RoadEditingPropertyStep::TableField {
            table: RoadEditingTableKind::CurveSegment,
            field_id: 1,
        }]));
        let context = Arc::new(RoadEditingLocationContext::new(
            Box::new([Arc::from("alignment-main"), Arc::from("city/main")]),
            Box::new([path]),
            Box::new([Arc::from("canvas/segment-0")]),
        ));
        let factory = RoadEditingLocationFactory {
            context,
            document_identity: RoadEditingDocumentIdentity::verified(
                Arc::from("city/main"),
                Arc::from("roads/main"),
            ),
        };

        let location = factory.road_alignment_owner_local(
            "alignment-main",
            RoadEditingRelationKind::CurveSegment,
            RoadEditingRelationOccurrence::OrderedProductOrdinal(0),
            &[RoadEditingPropertyStep::TableField {
                table: RoadEditingTableKind::CurveSegment,
                field_id: 1,
            }],
            Some("canvas/segment-0"),
        );
        let road = location.road_editing().expect("road-editing location");

        assert_eq!(road.canvas_selection(), Some("canvas/segment-0"));
        let RoadEditingSubject::OwnerLocal {
            owner: RoadEditingOwner::Address(address),
            ..
        } = road.subject()
        else {
            panic!("road-alignment owner-local subject expected");
        };
        assert_eq!(address.kind(), RoadEditingAddressKind::RoadAlignment);
        assert_eq!(address.local_key(road.context()), "alignment-main");
    }
}
