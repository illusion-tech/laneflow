//! 受检 RoadEditingSource wire 到共同 Typed AST 的零歧义引用降阶。

use std::cmp::Ordering;
use std::sync::Arc;

use laneflow_road_editing_wire::generated::lane_flow::road_editing::v1 as wire;
use laneflow_static_contract::{EntityKind, EntityKindMarker, LaneEdgeKind, ParticipantClassKind};

use super::location::RoadEditingLocationFactory;
use super::rules::validate_wire_reference;
use crate::declaration::{
    CanonicalFrameDeclaration, DeclarationHeader, IidmVehicleProfileInput, OwnedEntityReference,
    ParkingAreaDeclaration, ParticipantClassDeclaration, SignalGroupDeclaration,
    StaticRouteDeclaration, StopLineDeclaration, TypedAstDeclaration, TypedAstEntityAddress,
    VehicleProfileDeclaration,
};
use crate::{
    RoadEditingPropertyStep, RoadEditingRelationKind, RoadEditingRelationOccurrence,
    RoadEditingTableKind, SourceLocation,
};

const MAX_OWNER_QUALIFIED_COMPONENTS: usize = 4;

/// 受检 wire reference 的借用规范视图；固定数组避免为比较和排序建立临时 key 字符串。
#[derive(Clone, Copy, Debug)]
struct BorrowedReference<'a> {
    module_namespace: &'a str,
    components: [&'a str; MAX_OWNER_QUALIFIED_COMPONENTS],
    component_count: u8,
}

impl<'a> BorrowedReference<'a> {
    fn parse(value: &'a str, component_count: u8, current_namespace: &'a str) -> Self {
        let parsed = validate_wire_reference(value, component_count, true)
            .expect("semantic preflight validated every reference before lowering");
        let mut components = [""; MAX_OWNER_QUALIFIED_COMPONENTS];
        for (index, component) in parsed.key_components().enumerate() {
            components[index] = component;
        }
        Self {
            module_namespace: parsed.namespace().unwrap_or(current_namespace),
            components,
            component_count,
        }
    }

    fn owner_local_keys(&self) -> &[&'a str] {
        &self.components[..usize::from(self.component_count - 1)]
    }

    fn local_key(self) -> &'a str {
        self.components[usize::from(self.component_count - 1)]
    }

    fn cmp(self, other: Self) -> Ordering {
        self.module_namespace
            .as_bytes()
            .cmp(other.module_namespace.as_bytes())
            .then_with(|| {
                self.components[..usize::from(self.component_count)]
                    .iter()
                    .map(|component| component.as_bytes())
                    .cmp(
                        other.components[..usize::from(other.component_count)]
                            .iter()
                            .map(|component| component.as_bytes()),
                    )
            })
    }
}

/// 把受检引用降为共同 Typed AST 地址；字符串只为最终 retained 记录分配一次。
fn lower_reference<K: EntityKindMarker>(
    value: &str,
    component_count: u8,
    current_namespace: &str,
    span: SourceLocation,
) -> OwnedEntityReference<K> {
    let reference = BorrowedReference::parse(value, component_count, current_namespace);
    let owner_local_keys: Arc<[Arc<str>]> = reference
        .owner_local_keys()
        .iter()
        .copied()
        .map(Arc::from)
        .collect();
    let local_key = Arc::<str>::from(reference.local_key());
    let target_address = if owner_local_keys.is_empty() {
        TypedAstEntityAddress::module_scoped(local_key)
    } else {
        TypedAstEntityAddress::owner_scoped(owner_local_keys, local_key)
    };
    OwnedEntityReference::with_target_address(
        Arc::from(reference.module_namespace),
        target_address,
        span,
    )
}

/// 按解析后的命名空间与完整 owner tuple 规范排序无序引用集合。
fn sort_reference_set(values: &mut [&str], component_count: u8, current_namespace: &str) {
    values.sort_unstable_by(|left, right| {
        BorrowedReference::parse(left, component_count, current_namespace).cmp(
            BorrowedReference::parse(right, component_count, current_namespace),
        )
    });
}

/// 先降阶不依赖几何载荷或 owner 聚合的独立声明族。
fn lower_independent_declarations(
    root: wire::RoadEditingSource<'_>,
    locations: &RoadEditingLocationFactory,
) -> Vec<TypedAstDeclaration> {
    let mut declarations = Vec::with_capacity(
        root.signal_groups()
            .len()
            .saturating_add(root.parking_areas().len())
            .saturating_add(root.participant_classes().len())
            .saturating_add(root.vehicle_profiles().len())
            .saturating_add(root.stop_lines().len())
            .saturating_add(root.static_routes().len())
            .saturating_add(root.canonical_frames().len()),
    );

    let mut signal_groups: Vec<_> = root.signal_groups().iter().collect();
    signal_groups.sort_unstable_by(|left, right| {
        left.signal_group_key()
            .as_bytes()
            .cmp(right.signal_group_key().as_bytes())
    });
    declarations.extend(signal_groups.into_iter().map(|value| {
        TypedAstDeclaration::SignalGroup(SignalGroupDeclaration {
            header: module_scoped_header(
                locations,
                EntityKind::SignalGroup,
                value.signal_group_key(),
                value.canvas_selection(),
            ),
        })
    }));

    let mut parking_areas: Vec<_> = root.parking_areas().iter().collect();
    parking_areas.sort_unstable_by(|left, right| {
        left.parking_area_key()
            .as_bytes()
            .cmp(right.parking_area_key().as_bytes())
    });
    declarations.extend(parking_areas.into_iter().map(|value| {
        TypedAstDeclaration::ParkingArea(ParkingAreaDeclaration {
            header: module_scoped_header(
                locations,
                EntityKind::ParkingArea,
                value.parking_area_key(),
                value.canvas_selection(),
            ),
        })
    }));

    let namespace = root.module_header().authoring_namespace_id();
    let mut participant_classes: Vec<_> = root.participant_classes().iter().collect();
    participant_classes.sort_unstable_by(|left, right| {
        left.participant_class_key()
            .as_bytes()
            .cmp(right.participant_class_key().as_bytes())
    });
    declarations.extend(participant_classes.into_iter().map(|value| {
        let key = value.participant_class_key();
        TypedAstDeclaration::ParticipantClass(ParticipantClassDeclaration {
            header: module_scoped_header(
                locations,
                EntityKind::ParticipantClass,
                key,
                value.canvas_selection(),
            ),
            extends: value.extends().map(|parent| {
                lower_reference::<ParticipantClassKind>(
                    parent,
                    1,
                    namespace,
                    property_location(
                        locations,
                        EntityKind::ParticipantClass,
                        key,
                        RoadEditingTableKind::ParticipantClass,
                        1,
                        value.canvas_selection(),
                    ),
                )
            }),
        })
    }));

    let mut vehicle_profiles: Vec<_> = root.vehicle_profiles().iter().collect();
    vehicle_profiles.sort_unstable_by(|left, right| {
        left.vehicle_profile_key()
            .as_bytes()
            .cmp(right.vehicle_profile_key().as_bytes())
    });
    declarations.extend(vehicle_profiles.into_iter().map(|value| {
        let key = value.vehicle_profile_key();
        let iidm = value.iidm();
        TypedAstDeclaration::VehicleProfile(VehicleProfileDeclaration {
            header: module_scoped_header(
                locations,
                EntityKind::VehicleProfile,
                key,
                value.canvas_selection(),
            ),
            participant_class: lower_reference::<ParticipantClassKind>(
                value.participant_class(),
                1,
                namespace,
                property_location(
                    locations,
                    EntityKind::VehicleProfile,
                    key,
                    RoadEditingTableKind::VehicleProfile,
                    1,
                    value.canvas_selection(),
                ),
            ),
            iidm: IidmVehicleProfileInput {
                length_meters: iidm.length_meters(),
                desired_speed_meters_per_second: iidm.desired_speed_meters_per_second(),
                min_gap_meters: iidm.min_gap_meters(),
                time_headway_seconds: iidm.time_headway_seconds(),
                max_acceleration_meters_per_second_squared: iidm
                    .max_acceleration_meters_per_second_squared(),
                comfortable_deceleration_meters_per_second_squared: iidm
                    .comfortable_deceleration_meters_per_second_squared(),
                emergency_deceleration_meters_per_second_squared: iidm
                    .emergency_deceleration_meters_per_second_squared(),
            },
        })
    }));

    let mut stop_lines: Vec<_> = root.stop_lines().iter().collect();
    stop_lines.sort_unstable_by(|left, right| {
        left.stop_line_key()
            .as_bytes()
            .cmp(right.stop_line_key().as_bytes())
    });
    declarations.extend(stop_lines.into_iter().map(|value| {
        let key = value.stop_line_key();
        TypedAstDeclaration::StopLine(StopLineDeclaration {
            header: module_scoped_header(
                locations,
                EntityKind::StopLine,
                key,
                value.canvas_selection(),
            ),
            lane_edge: lower_reference::<LaneEdgeKind>(
                value.lane_edge(),
                1,
                namespace,
                property_location(
                    locations,
                    EntityKind::StopLine,
                    key,
                    RoadEditingTableKind::StopLine,
                    1,
                    value.canvas_selection(),
                ),
            ),
        })
    }));

    let mut static_routes: Vec<_> = root.static_routes().iter().collect();
    static_routes.sort_unstable_by(|left, right| {
        left.static_route_key()
            .as_bytes()
            .cmp(right.static_route_key().as_bytes())
    });
    declarations.extend(static_routes.into_iter().map(|value| {
        let key = value.static_route_key();
        let edge_sequence = value
            .edge_sequence()
            .iter()
            .enumerate()
            .map(|(index, edge)| {
                lower_reference::<LaneEdgeKind>(
                    edge,
                    1,
                    namespace,
                    locations.owner_local(
                        EntityKind::StaticRoute,
                        &[],
                        key,
                        RoadEditingRelationKind::StaticRouteEdge,
                        RoadEditingRelationOccurrence::OrderedProductOrdinal(
                            u32::try_from(index).expect("compile limits bound relation ordinals"),
                        ),
                        &[RoadEditingPropertyStep::TableField {
                            table: RoadEditingTableKind::StaticRoute,
                            field_id: 1,
                        }],
                    ),
                )
            })
            .collect();
        TypedAstDeclaration::StaticRoute(StaticRouteDeclaration {
            header: module_scoped_header(
                locations,
                EntityKind::StaticRoute,
                key,
                value.canvas_selection(),
            ),
            edge_sequence,
        })
    }));

    let mut canonical_frames: Vec<_> = root.canonical_frames().iter().collect();
    canonical_frames.sort_unstable_by(|left, right| {
        left.canonical_frame_key()
            .as_bytes()
            .cmp(right.canonical_frame_key().as_bytes())
    });
    declarations.extend(canonical_frames.into_iter().map(|value| {
        TypedAstDeclaration::CanonicalFrame(CanonicalFrameDeclaration {
            header: module_scoped_header(
                locations,
                EntityKind::CanonicalFrame,
                value.canonical_frame_key(),
                value.canvas_selection(),
            ),
            lane_edge_geometries: Box::new([]),
        })
    }));

    declarations
}

fn module_scoped_header(
    locations: &RoadEditingLocationFactory,
    entity_kind: EntityKind,
    local_key: &str,
    canvas_selection: Option<&str>,
) -> DeclarationHeader {
    DeclarationHeader::module_scoped(
        entity_kind,
        Arc::from(local_key),
        locations.declaration(entity_kind, &[], local_key, canvas_selection),
    )
}

fn property_location(
    locations: &RoadEditingLocationFactory,
    entity_kind: EntityKind,
    local_key: &str,
    table: RoadEditingTableKind,
    field_id: u16,
    canvas_selection: Option<&str>,
) -> SourceLocation {
    locations.property(
        entity_kind,
        &[],
        local_key,
        &[RoadEditingPropertyStep::TableField { table, field_id }],
        canvas_selection,
    )
}

#[cfg(test)]
mod tests {
    use laneflow_static_contract::{ManeuverGateKind, SignalGroupKind};

    use super::*;
    use crate::road_editing::{
        CanonicalFrameInput, IidmVehicleProfileInput as RoadEditingIidmInput, LaneEdgeReference,
        ParkingAreaInput, ParticipantClassInput, ParticipantClassReference, RoadEditingDeclaration,
        RoadEditingModuleHeader, RoadEditingModuleInput, RoadEditingProvenance,
        RoadEditingSourceModuleBuilder, RoadEditingSourceWriter, StaticRouteInput, StopLineInput,
        VehicleProfileInput,
    };
    use crate::{CompileLimits, GeometryAccuracyProfile, GeometryDirectionProfile, SourceSpan};

    #[test]
    fn owner_qualified_reference_lowers_without_flattening_the_address() {
        let reference = lower_reference::<ManeuverGateKind>(
            "city/base::junction-main>movement-left>path-main>gate-entry",
            4,
            "city/current",
            SourceSpan::point(Arc::from("roads/main"), 1, 1).into(),
        );

        assert_eq!(reference.module_namespace.as_ref(), "city/base");
        assert_eq!(
            reference
                .target_address
                .owner_local_keys()
                .iter()
                .map(AsRef::as_ref)
                .collect::<Vec<_>>(),
            ["junction-main", "movement-left", "path-main"]
        );
        assert_eq!(reference.declaration_key().as_ref(), "gate-entry");
    }

    #[test]
    fn unordered_reference_sort_uses_resolved_namespace_and_key_bytes() {
        let mut values = ["city/z::group-a", "group-z", "city/a::group-z", "group-a"];
        sort_reference_set(&mut values, 1, "city/main");

        assert_eq!(
            values,
            ["city/a::group-z", "group-a", "group-z", "city/z::group-a"]
        );

        let local = lower_reference::<SignalGroupKind>(
            values[1],
            1,
            "city/main",
            SourceSpan::point(Arc::from("roads/main"), 1, 1).into(),
        );
        assert_eq!(local.module_namespace.as_ref(), "city/main");
    }

    #[test]
    fn independent_declarations_are_lowered_in_stable_key_order() {
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
        for declaration in [
            RoadEditingDeclaration::CanonicalFrame(
                CanonicalFrameInput::try_new("frame-z").unwrap(),
            ),
            RoadEditingDeclaration::ParkingArea(ParkingAreaInput::try_new("parking-z").unwrap()),
            RoadEditingDeclaration::CanonicalFrame(
                CanonicalFrameInput::try_new("frame-a").unwrap(),
            ),
            RoadEditingDeclaration::ParkingArea(ParkingAreaInput::try_new("parking-a").unwrap()),
        ] {
            builder.add_declaration(declaration).unwrap();
        }
        let bytes = RoadEditingSourceWriter::new(&limits)
            .write(builder.finish().unwrap())
            .unwrap();
        let input = RoadEditingModuleInput::try_new("roads/main", bytes.as_bytes(), None).unwrap();
        let verified = super::super::reader::verify_source(input, &limits, 0, 0).unwrap();
        let locations = RoadEditingLocationFactory::from_verified_root(verified.root());
        let declarations = lower_independent_declarations(verified.root(), &locations);

        assert_eq!(declarations.len(), 4);
        assert!(matches!(
            &declarations[0],
            TypedAstDeclaration::ParkingArea(value)
                if value.header.stable_key.as_ref() == "parking-a"
        ));
        assert!(matches!(
            &declarations[1],
            TypedAstDeclaration::ParkingArea(value)
                if value.header.stable_key.as_ref() == "parking-z"
        ));
        assert!(matches!(
            &declarations[2],
            TypedAstDeclaration::CanonicalFrame(value)
                if value.header.stable_key.as_ref() == "frame-a"
        ));
        assert!(matches!(
            &declarations[3],
            TypedAstDeclaration::CanonicalFrame(value)
                if value.header.stable_key.as_ref() == "frame-z"
        ));
    }

    #[test]
    fn independent_reference_declarations_preserve_imports_and_ordered_occurrences() {
        let limits = CompileLimits::p100_initial_v2();
        let header = RoadEditingModuleHeader::try_new(
            "city/main",
            "roads/main",
            vec!["city/base".to_owned()],
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
        let parent =
            ParticipantClassReference::imported("city/base", Vec::new(), "participant-base")
                .unwrap();
        let participant = ParticipantClassInput::try_new("participant-car")
            .unwrap()
            .with_extends(parent);
        builder
            .add_declaration(RoadEditingDeclaration::ParticipantClass(participant))
            .unwrap();
        builder
            .add_declaration(RoadEditingDeclaration::VehicleProfile(
                VehicleProfileInput::try_new(
                    "vehicle-car",
                    ParticipantClassReference::local("participant-car").unwrap(),
                    RoadEditingIidmInput::try_new(4.5, 15.0, 2.0, 1.5, 2.0, 3.0, 6.0).unwrap(),
                )
                .unwrap(),
            ))
            .unwrap();
        let imported_edge =
            LaneEdgeReference::imported("city/base", Vec::new(), "edge-main").unwrap();
        builder
            .add_declaration(RoadEditingDeclaration::StopLine(
                StopLineInput::try_new("stop-main", imported_edge.clone()).unwrap(),
            ))
            .unwrap();
        builder
            .add_declaration(RoadEditingDeclaration::StaticRoute(
                StaticRouteInput::try_new("route-main", vec![imported_edge.clone(), imported_edge])
                    .unwrap(),
            ))
            .unwrap();

        let bytes = RoadEditingSourceWriter::new(&limits)
            .write(builder.finish().unwrap())
            .unwrap();
        let input = RoadEditingModuleInput::try_new("roads/main", bytes.as_bytes(), None).unwrap();
        let verified = super::super::reader::verify_source(input, &limits, 0, 0).unwrap();
        let locations = RoadEditingLocationFactory::from_verified_root(verified.root());
        let declarations = lower_independent_declarations(verified.root(), &locations);

        let TypedAstDeclaration::ParticipantClass(participant) = &declarations[0] else {
            panic!("expected participant class");
        };
        assert_eq!(
            participant
                .extends
                .as_ref()
                .unwrap()
                .module_namespace
                .as_ref(),
            "city/base"
        );
        let TypedAstDeclaration::StaticRoute(route) = &declarations[3] else {
            panic!("expected static route");
        };
        assert_eq!(route.edge_sequence.len(), 2);
        assert!(matches!(
            route.edge_sequence[1]
                .span
                .road_editing()
                .unwrap()
                .subject(),
            crate::RoadEditingSubject::OwnerLocal {
                relation: RoadEditingRelationKind::StaticRouteEdge,
                occurrence: RoadEditingRelationOccurrence::OrderedProductOrdinal(1),
                ..
            }
        ));
    }
}
