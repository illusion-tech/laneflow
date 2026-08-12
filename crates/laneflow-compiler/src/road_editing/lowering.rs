//! 受检 RoadEditingSource wire 到共同 Typed AST 的零歧义引用降阶。

use std::cmp::Ordering;
use std::sync::Arc;

use laneflow_road_editing_wire::generated::lane_flow::road_editing::v1 as wire;
use laneflow_static_contract::{
    AccessEffect, AuthoringLaneKind, CanonicalFrameKind, EntityKind, EntityKindMarker,
    FacilityBandKind, JunctionKind, LaneEdgeKind, LaneGroupKind, ManeuverGateKind,
    ManeuverPathKind, MovementKind, ParkingAreaKind, ParticipantClassKind, RoadSectionKind,
    SignalAspect, SignalGroupKind, StopLineKind,
};

use super::location::RoadEditingLocationFactory;
use super::rules::validate_wire_reference;
use crate::declaration::{
    AccessRuleDeclaration, AuthoringCurveProgramDeclaration, AuthoringCurveSegmentDeclaration,
    AuthoringCurveSegmentGeometry, AuthoringLaneDeclaration, AuthoringLaneDirection,
    AuthoringLaneGeometry, AuthoringPoint3F64, AuthoringStationEnd, AuthoringWidthProfile,
    CanonicalFrameDeclaration, DeclarationHeader, FacilityBandDeclaration, IidmVehicleProfileInput,
    JunctionDeclaration, LaneEdgeDeclaration, LaneEdgeGeometryAuthority, LaneGroupDeclaration,
    ManeuverGateDeclaration, ManeuverPathDeclaration, MovementDeclaration, OwnedAccessRegulation,
    OwnedAccessRuleTarget, OwnedCorridorElementReference, OwnedEntityReference, OwnedSignalControl,
    ParkingAreaDeclaration, ParkingLaneAnchorDeclaration, ParkingSpaceDeclaration,
    ParkingSpaceGeometryInput, ParticipantClassDeclaration, RoadAlignmentDeclaration,
    RoadCorridorAuthoringGeometry, RoadCorridorDeclaration, RoadSectionDeclaration,
    SignalControllerDeclaration, SignalGroupDeclaration, SignalGroupStateDeclaration,
    SignalPhaseDeclaration, SpeedLimit, StaticRouteDeclaration, StopLineDeclaration,
    TypedAstDeclaration, TypedAstEntityAddress, VehicleProfileDeclaration, WaitingZoneDeclaration,
};
use crate::{
    RoadEditingPropertyStep, RoadEditingRelationKind, RoadEditingRelationOccurrence,
    RoadEditingTableKind, SourceLocation,
};

const MAX_OWNER_QUALIFIED_COMPONENTS: usize = 4;

pub(super) fn lower_road_alignments(
    root: wire::RoadEditingSource<'_>,
    locations: &RoadEditingLocationFactory,
    shared_namespace: &Arc<str>,
) -> Vec<RoadAlignmentDeclaration> {
    let mut values: Vec<_> = root.road_alignments().iter().collect();
    values.sort_unstable_by(|left, right| {
        left.road_alignment_key()
            .as_bytes()
            .cmp(right.road_alignment_key().as_bytes())
    });
    values
        .into_iter()
        .map(|value| {
            let key = value.road_alignment_key();
            RoadAlignmentDeclaration {
                road_alignment_key: Arc::from(key),
                canonical_frame: lower_reference::<CanonicalFrameKind>(
                    value.canonical_frame(),
                    1,
                    shared_namespace,
                    locations.road_alignment_property(
                        key,
                        &[RoadEditingPropertyStep::TableField {
                            table: RoadEditingTableKind::RoadAlignment,
                            field_id: 1,
                        }],
                        value.canvas_selection(),
                    ),
                ),
                reference_line: lower_curve_program(
                    value.reference_line(),
                    locations.road_alignment_property(
                        key,
                        &[
                            RoadEditingPropertyStep::TableField {
                                table: RoadEditingTableKind::RoadAlignment,
                                field_id: 2,
                            },
                            RoadEditingPropertyStep::TableField {
                                table: RoadEditingTableKind::CurveProgram,
                                field_id: 0,
                            },
                        ],
                        value.canvas_selection(),
                    ),
                    |index, canvas_selection| {
                        locations.road_alignment_owner_local(
                            key,
                            RoadEditingRelationKind::CurveSegment,
                            RoadEditingRelationOccurrence::OrderedProductOrdinal(
                                u32::try_from(index)
                                    .expect("compile limits bound curve segment ordinals"),
                            ),
                            &[RoadEditingPropertyStep::TableField {
                                table: RoadEditingTableKind::CurveSegment,
                                field_id: 1,
                            }],
                            canvas_selection,
                        )
                    },
                ),
                span: locations.road_alignment(key, value.canvas_selection()),
            }
        })
        .collect()
}

fn lower_curve_program(
    value: wire::CurveProgram<'_>,
    start_span: SourceLocation,
    mut segment_span: impl FnMut(usize, Option<&str>) -> SourceLocation,
) -> AuthoringCurveProgramDeclaration {
    let start = lower_point(value.start());
    let segments = value
        .segments()
        .iter()
        .enumerate()
        .map(|(index, segment)| {
            let geometry = match segment.geometry_type() {
                wire::CurveSegmentGeometry::LineSegment => {
                    let geometry = segment
                        .geometry_as_line_segment()
                        .expect("semantic preflight validated curve union payloads");
                    AuthoringCurveSegmentGeometry::Line {
                        end: lower_point(geometry.end()),
                    }
                }
                wire::CurveSegmentGeometry::CubicBezierSegment => {
                    let geometry = segment
                        .geometry_as_cubic_bezier_segment()
                        .expect("semantic preflight validated curve union payloads");
                    AuthoringCurveSegmentGeometry::CubicBezier {
                        control_1: lower_point(geometry.control_1()),
                        control_2: lower_point(geometry.control_2()),
                        end: lower_point(geometry.end()),
                    }
                }
                _ => unreachable!("semantic preflight validated curve union discriminants"),
            };
            AuthoringCurveSegmentDeclaration {
                geometry,
                span: segment_span(index, segment.canvas_selection()),
            }
        })
        .collect();
    AuthoringCurveProgramDeclaration {
        start,
        start_span,
        segments,
    }
}

fn lower_point(value: &wire::Vec3F64) -> AuthoringPoint3F64 {
    AuthoringPoint3F64 {
        x: canonicalize_zero(value.x()),
        y: canonicalize_zero(value.y()),
        z: canonicalize_zero(value.z()),
    }
}

/// 受检 wire reference 的借用规范视图；固定数组避免为比较和排序建立临时 key 字符串。
#[derive(Clone, Copy, Debug)]
struct BorrowedReference<'a> {
    module_namespace: &'a str,
    uses_current_namespace: bool,
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
            uses_current_namespace: parsed.namespace().is_none(),
            components,
            component_count,
        }
    }

    fn owner_local_keys(&self) -> &[&'a str] {
        &self.components[..usize::from(self.component_count - 1)]
    }

    fn owner_local_keys_with_local(&self) -> &[&'a str] {
        &self.components[..usize::from(self.component_count)]
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
    current_namespace: &Arc<str>,
    span: SourceLocation,
) -> OwnedEntityReference<K> {
    let reference = BorrowedReference::parse(value, component_count, current_namespace.as_ref());
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
    let module_namespace = if reference.uses_current_namespace {
        Arc::clone(current_namespace)
    } else {
        Arc::from(reference.module_namespace)
    };
    OwnedEntityReference::with_target_address(module_namespace, target_address, span)
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
pub(super) fn lower_independent_declarations(
    root: wire::RoadEditingSource<'_>,
    locations: &RoadEditingLocationFactory,
    shared_namespace: &Arc<str>,
    declarations: &mut Vec<TypedAstDeclaration>,
) {
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
                    shared_namespace,
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
                shared_namespace,
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
                length_meters: canonicalize_zero(iidm.length_meters()),
                desired_speed_meters_per_second: canonicalize_zero(
                    iidm.desired_speed_meters_per_second(),
                ),
                min_gap_meters: canonicalize_zero(iidm.min_gap_meters()),
                time_headway_seconds: canonicalize_zero(iidm.time_headway_seconds()),
                max_acceleration_meters_per_second_squared: canonicalize_zero(
                    iidm.max_acceleration_meters_per_second_squared(),
                ),
                comfortable_deceleration_meters_per_second_squared: canonicalize_zero(
                    iidm.comfortable_deceleration_meters_per_second_squared(),
                ),
                emergency_deceleration_meters_per_second_squared: canonicalize_zero(
                    iidm.emergency_deceleration_meters_per_second_squared(),
                ),
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
                shared_namespace,
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
                    shared_namespace,
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
                        value.canvas_selection(),
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
}

/// 降阶以完整 owner tuple 定址、但不依赖几何点载荷或嵌套聚合的声明族。
pub(super) fn lower_owner_scoped_declarations(
    root: wire::RoadEditingSource<'_>,
    locations: &RoadEditingLocationFactory,
    shared_namespace: &Arc<str>,
    declarations: &mut Vec<TypedAstDeclaration>,
) {
    let namespace = shared_namespace.as_ref();

    let mut lane_groups: Vec<_> = root.lane_groups().iter().collect();
    lane_groups.sort_unstable_by(|left, right| {
        compare_owner_scoped_values(
            left.road_section(),
            left.lane_group_key(),
            right.road_section(),
            right.lane_group_key(),
            2,
            namespace,
        )
    });
    declarations.extend(lane_groups.into_iter().map(|value| {
        let key = value.lane_group_key();
        TypedAstDeclaration::LaneGroup(LaneGroupDeclaration {
            header: owner_scoped_header(
                locations,
                EntityKind::LaneGroup,
                value.road_section(),
                2,
                key,
                value.canvas_selection(),
                namespace,
            ),
            road_section: lower_reference::<RoadSectionKind>(
                value.road_section(),
                2,
                shared_namespace,
                owner_property_location(
                    locations,
                    EntityKind::LaneGroup,
                    value.road_section(),
                    2,
                    key,
                    RoadEditingTableKind::LaneGroup,
                    1,
                    value.canvas_selection(),
                    namespace,
                ),
            ),
        })
    }));

    let mut facility_bands: Vec<_> = root.facility_bands().iter().collect();
    facility_bands.sort_unstable_by(|left, right| {
        compare_owner_scoped_values(
            left.road_corridor(),
            left.facility_band_key(),
            right.road_corridor(),
            right.facility_band_key(),
            1,
            namespace,
        )
    });
    declarations.extend(facility_bands.into_iter().map(|value| {
        let key = value.facility_band_key();
        TypedAstDeclaration::FacilityBand(FacilityBandDeclaration {
            header: owner_scoped_header(
                locations,
                EntityKind::FacilityBand,
                value.road_corridor(),
                1,
                key,
                value.canvas_selection(),
                namespace,
            ),
            kind_id: Arc::from(value.kind_id()),
            authoring_width_profile: Some(AuthoringWidthProfile {
                start_width_meters: canonicalize_zero(value.width_profile().start_width_meters()),
                end_width_meters: canonicalize_zero(value.width_profile().end_width_meters()),
            }),
            compiled_geometry: None,
        })
    }));

    let mut junctions: Vec<_> = root.junctions().iter().collect();
    junctions.sort_unstable_by(|left, right| {
        left.junction_key()
            .as_bytes()
            .cmp(right.junction_key().as_bytes())
    });
    declarations.extend(junctions.into_iter().map(|value| {
        let key = value.junction_key();
        let mut approaches: Vec<_> = value.approach_edges().iter().collect();
        sort_reference_set(&mut approaches, 1, namespace);
        let approach_edges = approaches
            .into_iter()
            .enumerate()
            .map(|(index, edge)| {
                lower_reference::<LaneEdgeKind>(
                    edge,
                    1,
                    shared_namespace,
                    locations.owner_local(
                        EntityKind::Junction,
                        &[],
                        key,
                        RoadEditingRelationKind::JunctionApproachEdge,
                        RoadEditingRelationOccurrence::CanonicalSetOrdinal(
                            u32::try_from(index).expect("compile limits bound relation ordinals"),
                        ),
                        &[RoadEditingPropertyStep::TableField {
                            table: RoadEditingTableKind::Junction,
                            field_id: 1,
                        }],
                        value.canvas_selection(),
                    ),
                )
            })
            .collect();
        let mut internal: Vec<_> = value.internal_edges().iter().collect();
        sort_reference_set(&mut internal, 1, namespace);
        let internal_edges = internal
            .into_iter()
            .enumerate()
            .map(|(index, edge)| {
                lower_reference::<LaneEdgeKind>(
                    edge,
                    1,
                    shared_namespace,
                    locations.owner_local(
                        EntityKind::Junction,
                        &[],
                        key,
                        RoadEditingRelationKind::JunctionInternalEdge,
                        RoadEditingRelationOccurrence::CanonicalSetOrdinal(
                            u32::try_from(index).expect("compile limits bound relation ordinals"),
                        ),
                        &[RoadEditingPropertyStep::TableField {
                            table: RoadEditingTableKind::Junction,
                            field_id: 2,
                        }],
                        value.canvas_selection(),
                    ),
                )
            })
            .collect();
        TypedAstDeclaration::Junction(JunctionDeclaration {
            header: module_scoped_header(
                locations,
                EntityKind::Junction,
                key,
                value.canvas_selection(),
            ),
            approach_edges,
            internal_edges,
        })
    }));

    let mut movements: Vec<_> = root.movements().iter().collect();
    movements.sort_unstable_by(|left, right| {
        compare_owner_scoped_values(
            left.junction(),
            left.movement_key(),
            right.junction(),
            right.movement_key(),
            1,
            namespace,
        )
    });
    declarations.extend(movements.into_iter().map(|value| {
        let key = value.movement_key();
        TypedAstDeclaration::Movement(MovementDeclaration {
            header: owner_scoped_header(
                locations,
                EntityKind::Movement,
                value.junction(),
                1,
                key,
                value.canvas_selection(),
                namespace,
            ),
            junction: lower_reference::<JunctionKind>(
                value.junction(),
                1,
                shared_namespace,
                owner_property_location(
                    locations,
                    EntityKind::Movement,
                    value.junction(),
                    1,
                    key,
                    RoadEditingTableKind::Movement,
                    1,
                    value.canvas_selection(),
                    namespace,
                ),
            ),
            directed_entry_approach_key: Arc::from(value.directed_entry_approach_key()),
            directed_exit_approach_key: Arc::from(value.directed_exit_approach_key()),
        })
    }));

    let mut paths: Vec<_> = root.maneuver_paths().iter().collect();
    paths.sort_unstable_by(|left, right| {
        compare_owner_scoped_values(
            left.movement(),
            left.maneuver_path_key(),
            right.movement(),
            right.maneuver_path_key(),
            2,
            namespace,
        )
    });
    declarations.extend(paths.into_iter().map(|value| {
        let key = value.maneuver_path_key();
        let owner = BorrowedReference::parse(value.movement(), 2, namespace);
        let internal_edges = value
            .internal_edges()
            .iter()
            .enumerate()
            .map(|(index, edge)| {
                lower_reference::<LaneEdgeKind>(
                    edge,
                    1,
                    shared_namespace,
                    locations.owner_local(
                        EntityKind::ManeuverPath,
                        owner.owner_local_keys_with_local(),
                        key,
                        RoadEditingRelationKind::ManeuverPathInternalEdge,
                        RoadEditingRelationOccurrence::OrderedProductOrdinal(
                            u32::try_from(index).expect("compile limits bound relation ordinals"),
                        ),
                        &[RoadEditingPropertyStep::TableField {
                            table: RoadEditingTableKind::ManeuverPath,
                            field_id: 3,
                        }],
                        value.canvas_selection(),
                    ),
                )
            })
            .collect();
        TypedAstDeclaration::ManeuverPath(ManeuverPathDeclaration {
            header: owner_scoped_header(
                locations,
                EntityKind::ManeuverPath,
                value.movement(),
                2,
                key,
                value.canvas_selection(),
                namespace,
            ),
            movement: lower_reference::<MovementKind>(
                value.movement(),
                2,
                shared_namespace,
                owner_property_location(
                    locations,
                    EntityKind::ManeuverPath,
                    value.movement(),
                    2,
                    key,
                    RoadEditingTableKind::ManeuverPath,
                    1,
                    value.canvas_selection(),
                    namespace,
                ),
            ),
            entry_edge: lower_reference::<LaneEdgeKind>(
                value.entry_edge(),
                1,
                shared_namespace,
                owner_property_location(
                    locations,
                    EntityKind::ManeuverPath,
                    value.movement(),
                    2,
                    key,
                    RoadEditingTableKind::ManeuverPath,
                    2,
                    value.canvas_selection(),
                    namespace,
                ),
            ),
            internal_edges,
            exit_edge: lower_reference::<LaneEdgeKind>(
                value.exit_edge(),
                1,
                shared_namespace,
                owner_property_location(
                    locations,
                    EntityKind::ManeuverPath,
                    value.movement(),
                    2,
                    key,
                    RoadEditingTableKind::ManeuverPath,
                    4,
                    value.canvas_selection(),
                    namespace,
                ),
            ),
        })
    }));

    let mut gates: Vec<_> = root.maneuver_gates().iter().collect();
    gates.sort_unstable_by(|left, right| {
        compare_owner_scoped_values(
            left.maneuver_path(),
            left.maneuver_gate_key(),
            right.maneuver_path(),
            right.maneuver_gate_key(),
            3,
            namespace,
        )
    });
    declarations.extend(gates.into_iter().map(|value| {
        let key = value.maneuver_gate_key();
        let signal_control = match value.signal_control() {
            wire::SignalControlKind::None => OwnedSignalControl::None,
            wire::SignalControlKind::SignalGroup => {
                OwnedSignalControl::Group(lower_reference::<SignalGroupKind>(
                    value
                        .signal_group()
                        .expect("semantic preflight requires signal group for group control"),
                    1,
                    shared_namespace,
                    owner_property_location(
                        locations,
                        EntityKind::ManeuverGate,
                        value.maneuver_path(),
                        3,
                        key,
                        RoadEditingTableKind::ManeuverGate,
                        5,
                        value.canvas_selection(),
                        namespace,
                    ),
                ))
            }
            _ => unreachable!("semantic preflight rejects unspecified signal control"),
        };
        TypedAstDeclaration::ManeuverGate(ManeuverGateDeclaration {
            header: owner_scoped_header(
                locations,
                EntityKind::ManeuverGate,
                value.maneuver_path(),
                3,
                key,
                value.canvas_selection(),
                namespace,
            ),
            maneuver_path: lower_reference::<ManeuverPathKind>(
                value.maneuver_path(),
                3,
                shared_namespace,
                owner_property_location(
                    locations,
                    EntityKind::ManeuverGate,
                    value.maneuver_path(),
                    3,
                    key,
                    RoadEditingTableKind::ManeuverGate,
                    1,
                    value.canvas_selection(),
                    namespace,
                ),
            ),
            transition_index: value.transition_index(),
            stop_line: lower_reference::<StopLineKind>(
                value.stop_line(),
                1,
                shared_namespace,
                owner_property_location(
                    locations,
                    EntityKind::ManeuverGate,
                    value.maneuver_path(),
                    3,
                    key,
                    RoadEditingTableKind::ManeuverGate,
                    3,
                    value.canvas_selection(),
                    namespace,
                ),
            ),
            signal_control,
        })
    }));

    let mut waiting_zones: Vec<_> = root.waiting_zones().iter().collect();
    waiting_zones.sort_unstable_by(|left, right| {
        compare_owner_scoped_values(
            left.maneuver_path(),
            left.waiting_zone_key(),
            right.maneuver_path(),
            right.waiting_zone_key(),
            3,
            namespace,
        )
    });
    declarations.extend(waiting_zones.into_iter().map(|value| {
        let key = value.waiting_zone_key();
        TypedAstDeclaration::WaitingZone(WaitingZoneDeclaration {
            header: owner_scoped_header(
                locations,
                EntityKind::WaitingZone,
                value.maneuver_path(),
                3,
                key,
                value.canvas_selection(),
                namespace,
            ),
            maneuver_path: lower_reference::<ManeuverPathKind>(
                value.maneuver_path(),
                3,
                shared_namespace,
                owner_property_location(
                    locations,
                    EntityKind::WaitingZone,
                    value.maneuver_path(),
                    3,
                    key,
                    RoadEditingTableKind::WaitingZone,
                    1,
                    value.canvas_selection(),
                    namespace,
                ),
            ),
            entry_gate: lower_reference::<ManeuverGateKind>(
                value.entry_gate(),
                4,
                shared_namespace,
                owner_property_location(
                    locations,
                    EntityKind::WaitingZone,
                    value.maneuver_path(),
                    3,
                    key,
                    RoadEditingTableKind::WaitingZone,
                    2,
                    value.canvas_selection(),
                    namespace,
                ),
            ),
            release_gate: lower_reference::<ManeuverGateKind>(
                value.release_gate(),
                4,
                shared_namespace,
                owner_property_location(
                    locations,
                    EntityKind::WaitingZone,
                    value.maneuver_path(),
                    3,
                    key,
                    RoadEditingTableKind::WaitingZone,
                    3,
                    value.canvas_selection(),
                    namespace,
                ),
            ),
            max_occupancy: value.max_occupancy(),
        })
    }));
}

/// 降阶 RoadEditingSource 的道路横断面、LaneEdge 和 authoring 几何权威。
///
/// 该函数只转换已通过模块内 owner-tree 预检的值；跨模块引用、角色闭包、frame 派生和
/// 最终曲线编译仍由共同 HIR/topology-geometry 阶段一次完成。
pub(super) fn lower_topology_authoring_declarations(
    root: wire::RoadEditingSource<'_>,
    locations: &RoadEditingLocationFactory,
    shared_namespace: &Arc<str>,
    declarations: &mut Vec<TypedAstDeclaration>,
) -> Result<(), crate::DiagnosticBundle> {
    let namespace = shared_namespace.as_ref();
    let expected_key = root.module_header().source_document_key();

    let mut lane_edges: Vec<_> = root.lane_edges().iter().collect();
    lane_edges.sort_unstable_by(|left, right| {
        left.lane_edge_key()
            .as_bytes()
            .cmp(right.lane_edge_key().as_bytes())
    });
    declarations.extend(lane_edges.into_iter().map(|value| {
        let key = value.lane_edge_key();
        let mut successor_values: Vec<_> = value.successors().iter().collect();
        sort_reference_set(&mut successor_values, 1, namespace);
        let successors = successor_values
            .into_iter()
            .enumerate()
            .map(|(index, successor)| {
                lower_reference::<LaneEdgeKind>(
                    successor,
                    1,
                    shared_namespace,
                    locations.owner_local(
                        EntityKind::LaneEdge,
                        &[],
                        key,
                        RoadEditingRelationKind::LaneEdgeSuccessor,
                        RoadEditingRelationOccurrence::CanonicalSetOrdinal(
                            u32::try_from(index).expect("compile limits bound relation ordinals"),
                        ),
                        &[RoadEditingPropertyStep::TableField {
                            table: RoadEditingTableKind::LaneEdge,
                            field_id: 2,
                        }],
                        value.canvas_selection(),
                    ),
                )
            })
            .collect();
        let explicit_curve = value.explicit_geometry().map(|curve| {
            lower_curve_program(
                curve,
                nested_property_location(
                    locations,
                    EntityKind::LaneEdge,
                    key,
                    RoadEditingTableKind::LaneEdge,
                    3,
                    RoadEditingTableKind::CurveProgram,
                    0,
                    value.canvas_selection(),
                ),
                |index, canvas_selection| {
                    locations.owner_local(
                        EntityKind::LaneEdge,
                        &[],
                        key,
                        RoadEditingRelationKind::CurveSegment,
                        RoadEditingRelationOccurrence::OrderedProductOrdinal(
                            u32::try_from(index)
                                .expect("compile limits bound curve segment ordinals"),
                        ),
                        &[RoadEditingPropertyStep::TableField {
                            table: RoadEditingTableKind::CurveSegment,
                            field_id: 1,
                        }],
                        canvas_selection,
                    )
                },
            )
        });
        TypedAstDeclaration::LaneEdge(LaneEdgeDeclaration {
            header: module_scoped_header(
                locations,
                EntityKind::LaneEdge,
                key,
                value.canvas_selection(),
            ),
            geometry_authority: LaneEdgeGeometryAuthority::Authoring { explicit_curve },
            speed_limit: SpeedLimit::try_new(value.speed_limit_meters_per_second())
                .expect("semantic preflight validated lane speed"),
            successors,
        })
    }));

    let mut corridors: Vec<_> = root.road_corridors().iter().collect();
    corridors.sort_unstable_by(|left, right| {
        left.road_corridor_key()
            .as_bytes()
            .cmp(right.road_corridor_key().as_bytes())
    });
    declarations.extend(corridors.into_iter().map(|value| {
        let key = value.road_corridor_key();
        let elements = value
            .elements()
            .iter()
            .enumerate()
            .map(|(index, element)| {
                let span = locations.owner_local(
                    EntityKind::RoadCorridor,
                    &[],
                    key,
                    RoadEditingRelationKind::CorridorElement,
                    RoadEditingRelationOccurrence::OrderedProductOrdinal(
                        u32::try_from(index).expect("compile limits bound relation ordinals"),
                    ),
                    &[
                        RoadEditingPropertyStep::TableField {
                            table: RoadEditingTableKind::RoadCorridor,
                            field_id: 7,
                        },
                        RoadEditingPropertyStep::TableField {
                            table: RoadEditingTableKind::CorridorElement,
                            field_id: 1,
                        },
                    ],
                    value.canvas_selection(),
                );
                match element.kind() {
                    wire::CorridorElementKind::RoadSection => {
                        OwnedCorridorElementReference::RoadSection(
                            lower_reference::<RoadSectionKind>(
                                element.entity_reference(),
                                2,
                                shared_namespace,
                                span,
                            ),
                        )
                    }
                    wire::CorridorElementKind::FacilityBand => {
                        OwnedCorridorElementReference::FacilityBand(lower_reference::<
                            FacilityBandKind,
                        >(
                            element.entity_reference(),
                            2,
                            shared_namespace,
                            span,
                        ))
                    }
                    _ => unreachable!("semantic preflight validated corridor element kind"),
                }
            })
            .collect();
        let end_station = match value.end_station_kind() {
            wire::StationEndKind::Finite => {
                AuthoringStationEnd::Finite(canonicalize_zero(value.end_station_meters()))
            }
            wire::StationEndKind::AlignmentEnd => AuthoringStationEnd::AlignmentEnd,
            _ => unreachable!("semantic preflight validated station end kind"),
        };
        TypedAstDeclaration::RoadCorridor(RoadCorridorDeclaration {
            header: module_scoped_header(
                locations,
                EntityKind::RoadCorridor,
                key,
                value.canvas_selection(),
            ),
            reference_section: lower_reference::<RoadSectionKind>(
                value.reference_section(),
                2,
                shared_namespace,
                property_location(
                    locations,
                    EntityKind::RoadCorridor,
                    key,
                    RoadEditingTableKind::RoadCorridor,
                    5,
                    value.canvas_selection(),
                ),
            ),
            elements,
            authoring_geometry: Some(RoadCorridorAuthoringGeometry {
                road_alignment_key: Arc::from(value.road_alignment_key()),
                start_station_meters: canonicalize_zero(value.start_station_meters()),
                end_station,
                reference_lane: lower_reference::<AuthoringLaneKind>(
                    value.reference_lane(),
                    3,
                    shared_namespace,
                    property_location(
                        locations,
                        EntityKind::RoadCorridor,
                        key,
                        RoadEditingTableKind::RoadCorridor,
                        6,
                        value.canvas_selection(),
                    ),
                ),
            }),
        })
    }));

    let mut authoring_lanes: Vec<_> = root.authoring_lanes().iter().collect();
    authoring_lanes.sort_unstable_by(|left, right| {
        compare_owner_scoped_values(
            left.road_section(),
            left.authoring_lane_key(),
            right.road_section(),
            right.authoring_lane_key(),
            2,
            namespace,
        )
    });
    let expected_lane_count = authoring_lanes.len();
    let mut lowered_lane_count = 0_usize;
    let mut sections: Vec<_> = root.road_sections().iter().collect();
    sections.sort_unstable_by(|left, right| {
        compare_owner_scoped_values(
            left.road_corridor(),
            left.road_section_key(),
            right.road_corridor(),
            right.road_section_key(),
            1,
            namespace,
        )
    });
    for value in sections {
        let key = value.road_section_key();
        let owner = BorrowedReference::parse(value.road_corridor(), 1, namespace);
        let corridor_key = owner.local_key();
        let lanes = value
            .authoring_lanes()
            .iter()
            .enumerate()
            .map(|(index, lane_reference)| {
                let reference = BorrowedReference::parse(lane_reference, 3, namespace);
                if reference.module_namespace != namespace
                    || reference.owner_local_keys() != [corridor_key, key]
                {
                    return Err(super::preflight::invalid_combination_bundle(
                        "roadSection.authoringLanes",
                        expected_key,
                    ));
                }
                let lane_index = authoring_lanes
                    .binary_search_by(|lane| {
                        compare_owner_scoped_value_to_reference(
                            lane.road_section(),
                            lane.authoring_lane_key(),
                            2,
                            reference,
                            namespace,
                        )
                    })
                    .map_err(|_| {
                        super::preflight::invalid_combination_bundle(
                            "roadSection.authoringLanes",
                            expected_key,
                        )
                    })?;
                let lane = authoring_lanes[lane_index];
                let lane_key = lane.authoring_lane_key();
                Ok(AuthoringLaneDeclaration {
                    header: owner_scoped_header(
                        locations,
                        EntityKind::AuthoringLane,
                        lane.road_section(),
                        2,
                        lane_key,
                        lane.canvas_selection(),
                        namespace,
                    ),
                    section_relation_span: locations.owner_local(
                        EntityKind::RoadSection,
                        &[corridor_key],
                        key,
                        RoadEditingRelationKind::RoadSectionAuthoringLane,
                        RoadEditingRelationOccurrence::OrderedProductOrdinal(
                            u32::try_from(index).expect("compile limits bound relation ordinals"),
                        ),
                        &[RoadEditingPropertyStep::TableField {
                            table: RoadEditingTableKind::RoadSection,
                            field_id: 2,
                        }],
                        value.canvas_selection(),
                    ),
                    edge_chain: Box::new([lower_reference::<LaneEdgeKind>(
                        lane.lane_edge(),
                        1,
                        shared_namespace,
                        owner_property_location(
                            locations,
                            EntityKind::AuthoringLane,
                            lane.road_section(),
                            2,
                            lane_key,
                            RoadEditingTableKind::AuthoringLane,
                            1,
                            lane.canvas_selection(),
                            namespace,
                        ),
                    )]),
                    lane_group: lane.lane_group().map(|group| {
                        lower_reference::<LaneGroupKind>(
                            group,
                            3,
                            shared_namespace,
                            owner_property_location(
                                locations,
                                EntityKind::AuthoringLane,
                                lane.road_section(),
                                2,
                                lane_key,
                                RoadEditingTableKind::AuthoringLane,
                                4,
                                lane.canvas_selection(),
                                namespace,
                            ),
                        )
                    }),
                    authoring_geometry: Some(AuthoringLaneGeometry {
                        direction: match lane.direction() {
                            wire::LaneDirection::Forward => AuthoringLaneDirection::Forward,
                            wire::LaneDirection::Backward => AuthoringLaneDirection::Backward,
                            _ => unreachable!("semantic preflight validated lane direction"),
                        },
                        width_profile: AuthoringWidthProfile {
                            start_width_meters: canonicalize_zero(
                                lane.width_profile().start_width_meters(),
                            ),
                            end_width_meters: canonicalize_zero(
                                lane.width_profile().end_width_meters(),
                            ),
                        },
                    }),
                })
            })
            .collect::<Result<Box<[_]>, crate::DiagnosticBundle>>()?;
        lowered_lane_count = lowered_lane_count.saturating_add(lanes.len());
        declarations.push(TypedAstDeclaration::RoadSection(RoadSectionDeclaration {
            header: owner_scoped_header(
                locations,
                EntityKind::RoadSection,
                value.road_corridor(),
                1,
                key,
                value.canvas_selection(),
                namespace,
            ),
            kind_id: Arc::from(value.kind_id()),
            lanes,
        }));
    }
    if lowered_lane_count != expected_lane_count {
        return Err(super::preflight::invalid_combination_bundle(
            "authoringLanes.roadSection",
            expected_key,
        ));
    }

    Ok(())
}

/// 降阶需要把顶层声明与嵌套成员合并的非几何声明族。
///
/// semantic preflight 已闭合 SignalController/SignalPhase 双向所有权和状态完备性；这里
/// 只做确定性排序、来源位置绑定与共同 Typed AST 形状转换，不建立第二套语义规则。
pub(super) fn lower_aggregate_declarations(
    root: wire::RoadEditingSource<'_>,
    locations: &RoadEditingLocationFactory,
    shared_namespace: &Arc<str>,
    declarations: &mut Vec<TypedAstDeclaration>,
) {
    let namespace = shared_namespace.as_ref();

    let mut controllers: Vec<_> = root.signal_controllers().iter().collect();
    controllers.sort_unstable_by(|left, right| {
        left.signal_controller_key()
            .as_bytes()
            .cmp(right.signal_controller_key().as_bytes())
    });
    declarations.extend(controllers.into_iter().map(|controller| {
        let controller_key = controller.signal_controller_key();
        let mut group_references: Vec<_> = controller.signal_groups().iter().collect();
        sort_reference_set(&mut group_references, 1, namespace);
        let signal_groups = group_references
            .into_iter()
            .enumerate()
            .map(|(index, group)| {
                lower_reference::<SignalGroupKind>(
                    group,
                    1,
                    shared_namespace,
                    locations.owner_local(
                        EntityKind::SignalController,
                        &[],
                        controller_key,
                        RoadEditingRelationKind::SignalControllerGroup,
                        RoadEditingRelationOccurrence::CanonicalSetOrdinal(
                            u32::try_from(index).expect("compile limits bound relation ordinals"),
                        ),
                        &[RoadEditingPropertyStep::TableField {
                            table: RoadEditingTableKind::SignalController,
                            field_id: 2,
                        }],
                        controller.canvas_selection(),
                    ),
                )
            })
            .collect();

        let phases = controller
            .signal_phases()
            .iter()
            .enumerate()
            .map(|(phase_index, phase_reference)| {
                let phase_reference = BorrowedReference::parse(phase_reference, 2, namespace);
                debug_assert_eq!(phase_reference.module_namespace, namespace);
                debug_assert_eq!(phase_reference.owner_local_keys(), &[controller_key]);
                let phase_key = phase_reference.local_key();
                let phase = root
                    .signal_phases()
                    .iter()
                    .find(|phase| {
                        phase.signal_controller() == controller_key
                            && phase.signal_phase_key() == phase_key
                    })
                    .expect("semantic preflight closed controller/phase ownership");

                let mut states: Vec<_> = phase.states().iter().collect();
                states.sort_unstable_by(|left, right| {
                    BorrowedReference::parse(left.signal_group(), 1, namespace)
                        .cmp(BorrowedReference::parse(right.signal_group(), 1, namespace))
                });
                let states = states
                    .into_iter()
                    .enumerate()
                    .map(|(state_index, state)| SignalGroupStateDeclaration {
                        signal_group: lower_reference::<SignalGroupKind>(
                            state.signal_group(),
                            1,
                            shared_namespace,
                            locations.owner_local(
                                EntityKind::SignalPhase,
                                &[controller_key],
                                phase_key,
                                RoadEditingRelationKind::SignalPhaseState,
                                RoadEditingRelationOccurrence::CanonicalSetOrdinal(
                                    u32::try_from(state_index)
                                        .expect("compile limits bound relation ordinals"),
                                ),
                                &[
                                    RoadEditingPropertyStep::TableField {
                                        table: RoadEditingTableKind::SignalPhase,
                                        field_id: 2,
                                    },
                                    RoadEditingPropertyStep::TableField {
                                        table: RoadEditingTableKind::SignalPhaseState,
                                        field_id: 0,
                                    },
                                ],
                                phase.canvas_selection(),
                            ),
                        ),
                        aspect: match state.aspect() {
                            wire::SignalAspect::Red => SignalAspect::Red,
                            wire::SignalAspect::Yellow => SignalAspect::Yellow,
                            wire::SignalAspect::Green => SignalAspect::Green,
                            _ => unreachable!("semantic preflight rejects unknown signal aspect"),
                        },
                    })
                    .collect();

                SignalPhaseDeclaration {
                    header: owner_scoped_header(
                        locations,
                        EntityKind::SignalPhase,
                        phase.signal_controller(),
                        1,
                        phase_key,
                        phase.canvas_selection(),
                        namespace,
                    ),
                    controller_relation_span: locations.owner_local(
                        EntityKind::SignalController,
                        &[],
                        controller_key,
                        RoadEditingRelationKind::SignalControllerPhase,
                        RoadEditingRelationOccurrence::OrderedProductOrdinal(
                            u32::try_from(phase_index)
                                .expect("compile limits bound relation ordinals"),
                        ),
                        &[RoadEditingPropertyStep::TableField {
                            table: RoadEditingTableKind::SignalController,
                            field_id: 3,
                        }],
                        controller.canvas_selection(),
                    ),
                    duration_ms: phase.duration_milliseconds(),
                    states,
                }
            })
            .collect();

        TypedAstDeclaration::SignalController(SignalControllerDeclaration {
            header: module_scoped_header(
                locations,
                EntityKind::SignalController,
                controller_key,
                controller.canvas_selection(),
            ),
            offset_ms: controller.offset_milliseconds(),
            signal_groups,
            phases,
        })
    }));

    let mut parking_spaces: Vec<_> = root.parking_spaces().iter().collect();
    parking_spaces.sort_unstable_by(|left, right| {
        left.parking_space_key()
            .as_bytes()
            .cmp(right.parking_space_key().as_bytes())
    });
    declarations.extend(parking_spaces.into_iter().map(|value| {
        let key = value.parking_space_key();
        let entry = value.entry();
        let exit = value.exit();
        let geometry = value.geometry();
        TypedAstDeclaration::ParkingSpace(ParkingSpaceDeclaration {
            header: module_scoped_header(
                locations,
                EntityKind::ParkingSpace,
                key,
                value.canvas_selection(),
            ),
            parking_area: value.parking_area().map(|area| {
                lower_reference::<ParkingAreaKind>(
                    area,
                    1,
                    shared_namespace,
                    property_location(
                        locations,
                        EntityKind::ParkingSpace,
                        key,
                        RoadEditingTableKind::ParkingSpace,
                        1,
                        value.canvas_selection(),
                    ),
                )
            }),
            entry: ParkingLaneAnchorDeclaration {
                lane_edge: lower_reference::<LaneEdgeKind>(
                    entry.lane_edge(),
                    1,
                    shared_namespace,
                    nested_property_location(
                        locations,
                        EntityKind::ParkingSpace,
                        key,
                        RoadEditingTableKind::ParkingSpace,
                        2,
                        RoadEditingTableKind::ParkingLaneAnchor,
                        0,
                        value.canvas_selection(),
                    ),
                ),
                progress_meters: canonicalize_zero(entry.progress_meters()),
            },
            exit: ParkingLaneAnchorDeclaration {
                lane_edge: lower_reference::<LaneEdgeKind>(
                    exit.lane_edge(),
                    1,
                    shared_namespace,
                    nested_property_location(
                        locations,
                        EntityKind::ParkingSpace,
                        key,
                        RoadEditingTableKind::ParkingSpace,
                        3,
                        RoadEditingTableKind::ParkingLaneAnchor,
                        0,
                        value.canvas_selection(),
                    ),
                ),
                progress_meters: canonicalize_zero(exit.progress_meters()),
            },
            anchor_progress_spans: Box::new([
                nested_property_location(
                    locations,
                    EntityKind::ParkingSpace,
                    key,
                    RoadEditingTableKind::ParkingSpace,
                    2,
                    RoadEditingTableKind::ParkingLaneAnchor,
                    1,
                    value.canvas_selection(),
                ),
                nested_property_location(
                    locations,
                    EntityKind::ParkingSpace,
                    key,
                    RoadEditingTableKind::ParkingSpace,
                    3,
                    RoadEditingTableKind::ParkingLaneAnchor,
                    1,
                    value.canvas_selection(),
                ),
            ]),
            geometry: ParkingSpaceGeometryInput {
                lateral_offset_meters: canonicalize_zero(geometry.lateral_offset_meters()),
                heading_offset_radians: canonicalize_zero(geometry.heading_offset_radians()),
                length_meters: canonicalize_zero(geometry.length_meters()),
                width_meters: canonicalize_zero(geometry.width_meters()),
            },
        })
    }));

    let mut access_rules: Vec<_> = root.access_rules().iter().collect();
    access_rules.sort_unstable_by(|left, right| {
        left.access_rule_key()
            .as_bytes()
            .cmp(right.access_rule_key().as_bytes())
    });
    declarations.extend(access_rules.into_iter().map(|value| {
        let key = value.access_rule_key();
        let target_location = property_location(
            locations,
            EntityKind::AccessRule,
            key,
            RoadEditingTableKind::AccessRule,
            2,
            value.canvas_selection(),
        );
        let target = match value.target_kind() {
            wire::AccessTargetKind::LaneEdge => {
                OwnedAccessRuleTarget::LaneEdge(lower_reference::<LaneEdgeKind>(
                    value.target_reference(),
                    1,
                    shared_namespace,
                    target_location,
                ))
            }
            wire::AccessTargetKind::LaneGroup => {
                OwnedAccessRuleTarget::LaneGroup(lower_reference::<LaneGroupKind>(
                    value.target_reference(),
                    3,
                    shared_namespace,
                    target_location,
                ))
            }
            wire::AccessTargetKind::RoadSection => {
                OwnedAccessRuleTarget::RoadSection(lower_reference::<RoadSectionKind>(
                    value.target_reference(),
                    2,
                    shared_namespace,
                    target_location,
                ))
            }
            wire::AccessTargetKind::ManeuverPath => {
                OwnedAccessRuleTarget::ManeuverPath(lower_reference::<ManeuverPathKind>(
                    value.target_reference(),
                    3,
                    shared_namespace,
                    target_location,
                ))
            }
            _ => unreachable!("semantic preflight rejects unknown access target"),
        };

        let mut classes: Vec<_> = value.participant_classes().iter().collect();
        sort_reference_set(&mut classes, 1, namespace);
        let participant_classes = classes
            .into_iter()
            .enumerate()
            .map(|(index, class)| {
                lower_reference::<ParticipantClassKind>(
                    class,
                    1,
                    shared_namespace,
                    locations.owner_local(
                        EntityKind::AccessRule,
                        &[],
                        key,
                        RoadEditingRelationKind::AccessRuleParticipantClass,
                        RoadEditingRelationOccurrence::CanonicalSetOrdinal(
                            u32::try_from(index).expect("compile limits bound relation ordinals"),
                        ),
                        &[RoadEditingPropertyStep::TableField {
                            table: RoadEditingTableKind::AccessRule,
                            field_id: 4,
                        }],
                        value.canvas_selection(),
                    ),
                )
            })
            .collect();
        let regulation = value.regulation().map(|regulation| OwnedAccessRegulation {
            jurisdiction: Arc::from(regulation.jurisdiction()),
            version: Arc::from(regulation.version()),
            source: regulation.source().map(Arc::from),
        });

        TypedAstDeclaration::AccessRule(AccessRuleDeclaration {
            header: module_scoped_header(
                locations,
                EntityKind::AccessRule,
                key,
                value.canvas_selection(),
            ),
            target,
            effect: match value.effect() {
                wire::AccessEffect::Allow => AccessEffect::Allow,
                wire::AccessEffect::Deny => AccessEffect::Deny,
                _ => unreachable!("semantic preflight rejects unknown access effect"),
            },
            participant_classes,
            regulation,
            priority: value.priority(),
        })
    }));
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

#[allow(clippy::too_many_arguments)]
fn owner_scoped_header(
    locations: &RoadEditingLocationFactory,
    entity_kind: EntityKind,
    owner_reference: &str,
    owner_component_count: u8,
    local_key: &str,
    canvas_selection: Option<&str>,
    current_namespace: &str,
) -> DeclarationHeader {
    let owner = BorrowedReference::parse(owner_reference, owner_component_count, current_namespace);
    debug_assert_eq!(owner.module_namespace, current_namespace);
    let owner_keys: Arc<[Arc<str>]> = owner
        .owner_local_keys_with_local()
        .iter()
        .copied()
        .map(Arc::from)
        .collect();
    DeclarationHeader::with_source_address(
        entity_kind,
        TypedAstEntityAddress::owner_scoped(owner_keys, Arc::from(local_key)),
        Arc::from(local_key),
        locations.declaration(
            entity_kind,
            owner.owner_local_keys_with_local(),
            local_key,
            canvas_selection,
        ),
    )
}

fn compare_owner_scoped_values(
    left_owner: &str,
    left_key: &str,
    right_owner: &str,
    right_key: &str,
    owner_component_count: u8,
    current_namespace: &str,
) -> Ordering {
    BorrowedReference::parse(left_owner, owner_component_count, current_namespace)
        .cmp(BorrowedReference::parse(
            right_owner,
            owner_component_count,
            current_namespace,
        ))
        .then_with(|| left_key.as_bytes().cmp(right_key.as_bytes()))
}

fn compare_owner_scoped_value_to_reference(
    value_owner: &str,
    value_key: &str,
    owner_component_count: u8,
    reference: BorrowedReference<'_>,
    current_namespace: &str,
) -> Ordering {
    let owner = BorrowedReference::parse(value_owner, owner_component_count, current_namespace);
    owner
        .module_namespace
        .as_bytes()
        .cmp(reference.module_namespace.as_bytes())
        .then_with(|| {
            owner
                .owner_local_keys_with_local()
                .iter()
                .map(|component| component.as_bytes())
                .cmp(
                    reference
                        .owner_local_keys()
                        .iter()
                        .map(|component| component.as_bytes()),
                )
        })
        .then_with(|| value_key.as_bytes().cmp(reference.local_key().as_bytes()))
}

#[allow(clippy::too_many_arguments)]
fn owner_property_location(
    locations: &RoadEditingLocationFactory,
    entity_kind: EntityKind,
    owner_reference: &str,
    owner_component_count: u8,
    local_key: &str,
    table: RoadEditingTableKind,
    field_id: u16,
    canvas_selection: Option<&str>,
    current_namespace: &str,
) -> SourceLocation {
    let owner = BorrowedReference::parse(owner_reference, owner_component_count, current_namespace);
    locations.property(
        entity_kind,
        owner.owner_local_keys_with_local(),
        local_key,
        &[RoadEditingPropertyStep::TableField { table, field_id }],
        canvas_selection,
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

#[allow(clippy::too_many_arguments)]
fn nested_property_location(
    locations: &RoadEditingLocationFactory,
    entity_kind: EntityKind,
    local_key: &str,
    outer_table: RoadEditingTableKind,
    outer_field_id: u16,
    inner_table: RoadEditingTableKind,
    inner_field_id: u16,
    canvas_selection: Option<&str>,
) -> SourceLocation {
    locations.property(
        entity_kind,
        &[],
        local_key,
        &[
            RoadEditingPropertyStep::TableField {
                table: outer_table,
                field_id: outer_field_id,
            },
            RoadEditingPropertyStep::TableField {
                table: inner_table,
                field_id: inner_field_id,
            },
        ],
        canvas_selection,
    )
}

#[inline]
fn canonicalize_zero(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
}

#[cfg(test)]
mod tests {
    use laneflow_static_contract::{ManeuverGateKind, SignalGroupKind};

    use super::*;
    use crate::road_editing::{
        CanonicalFrameInput, IidmVehicleProfileInput as RoadEditingIidmInput, JunctionInput,
        JunctionReference, LaneEdgeInput, LaneEdgeReference, ManeuverPathInput, MovementInput,
        MovementReference, ParkingAreaInput, ParticipantClassInput, ParticipantClassReference,
        RoadEditingDeclaration, RoadEditingModuleHeader, RoadEditingModuleInput,
        RoadEditingProvenance, RoadEditingSourceModuleBuilder, RoadEditingSourceWriter,
        StaticRouteInput, StopLineInput, VehicleProfileInput,
    };
    use crate::{CompileLimits, GeometryAccuracyProfile, GeometryDirectionProfile, SourceSpan};

    #[test]
    fn negative_zero_is_canonicalized_before_synthetic_lowering() {
        assert_eq!(canonicalize_zero(-0.0).to_bits(), 0.0_f64.to_bits());
        assert_eq!(canonicalize_zero(1.25).to_bits(), 1.25_f64.to_bits());
    }

    #[test]
    fn owner_qualified_reference_lowers_without_flattening_the_address() {
        let reference = lower_reference::<ManeuverGateKind>(
            "city/base::junction-main>movement-left>path-main>gate-entry",
            4,
            &Arc::from("city/current"),
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
    fn geometry_points_canonicalize_negative_zero_at_the_typed_ast_boundary() {
        let lowered = lower_point(&wire::Vec3F64::new(-0.0, -0.0, -0.0));

        assert_eq!(lowered.x.to_bits(), 0.0_f64.to_bits());
        assert_eq!(lowered.y.to_bits(), 0.0_f64.to_bits());
        assert_eq!(lowered.z.to_bits(), 0.0_f64.to_bits());
    }

    #[test]
    fn unordered_reference_sort_uses_resolved_namespace_and_key_bytes() {
        let mut values = ["city/z::group-a", "group-z", "city/a::group-z", "group-a"];
        sort_reference_set(&mut values, 1, "city/main");

        assert_eq!(
            values,
            ["city/a::group-z", "group-a", "group-z", "city/z::group-a"]
        );

        let shared_namespace = Arc::<str>::from("city/main");
        let local = lower_reference::<SignalGroupKind>(
            values[1],
            1,
            &shared_namespace,
            SourceSpan::point(Arc::from("roads/main"), 1, 1).into(),
        );
        let other_local = lower_reference::<SignalGroupKind>(
            values[2],
            1,
            &shared_namespace,
            SourceSpan::point(Arc::from("roads/main"), 1, 2).into(),
        );
        assert_eq!(local.module_namespace.as_ref(), "city/main");
        assert!(Arc::ptr_eq(
            &local.module_namespace,
            &other_local.module_namespace
        ));
    }

    #[test]
    fn local_reference_reuses_verified_document_namespace_allocation() {
        let source = RoadEditingLocationFactory::verified_module_header("city/main", "roads/main");
        let document_namespace = source
            .road_editing()
            .and_then(|location| location.document_identity().module_namespace_arc())
            .expect("verified RoadEditing source has a module namespace");

        let reference =
            lower_reference::<SignalGroupKind>("group-a", 1, &document_namespace, source);

        assert!(Arc::ptr_eq(
            &reference.module_namespace,
            &document_namespace
        ));
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
        let shared_namespace = Arc::from(verified.root().module_header().authoring_namespace_id());
        let mut declarations = Vec::new();
        lower_independent_declarations(
            verified.root(),
            &locations,
            &shared_namespace,
            &mut declarations,
        );

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
                    .unwrap()
                    .with_canvas_selection("canvas/route")
                    .unwrap(),
            ))
            .unwrap();

        let bytes = RoadEditingSourceWriter::new(&limits)
            .write(builder.finish().unwrap())
            .unwrap();
        let input = RoadEditingModuleInput::try_new("roads/main", bytes.as_bytes(), None).unwrap();
        let verified = super::super::reader::verify_source(input, &limits, 0, 0).unwrap();
        let locations = RoadEditingLocationFactory::from_verified_root(verified.root());
        let shared_namespace = Arc::from(verified.root().module_header().authoring_namespace_id());
        let mut declarations = Vec::new();
        lower_independent_declarations(
            verified.root(),
            &locations,
            &shared_namespace,
            &mut declarations,
        );

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
        assert_eq!(
            route.edge_sequence[1]
                .span
                .road_editing()
                .unwrap()
                .canvas_selection(),
            Some("canvas/route")
        );
    }

    #[test]
    fn owner_scoped_declarations_keep_complete_parent_tuples() {
        let limits = CompileLimits::p100_initial_v2();
        let module = super::super::writer::tests::module_with_every_declaration(&limits);
        let bytes = RoadEditingSourceWriter::new(&limits).write(module).unwrap();
        let input =
            RoadEditingModuleInput::try_new("road-editing", bytes.as_bytes(), None).unwrap();
        let verified = super::super::reader::verify_source(input, &limits, 0, 0).unwrap();
        let locations = RoadEditingLocationFactory::from_verified_root(verified.root());
        let shared_namespace = Arc::from(verified.root().module_header().authoring_namespace_id());
        let mut independent = Vec::new();
        lower_independent_declarations(
            verified.root(),
            &locations,
            &shared_namespace,
            &mut independent,
        );
        let mut declarations = Vec::new();
        lower_owner_scoped_declarations(
            verified.root(),
            &locations,
            &shared_namespace,
            &mut declarations,
        );

        assert_eq!(declarations.len(), 7);
        let owners = declarations
            .iter()
            .filter_map(|declaration| match declaration {
                TypedAstDeclaration::LaneGroup(value) => Some((
                    EntityKind::LaneGroup,
                    value.header.source_address.owner_local_keys(),
                )),
                TypedAstDeclaration::FacilityBand(value) => Some((
                    EntityKind::FacilityBand,
                    value.header.source_address.owner_local_keys(),
                )),
                TypedAstDeclaration::Movement(value) => Some((
                    EntityKind::Movement,
                    value.header.source_address.owner_local_keys(),
                )),
                TypedAstDeclaration::ManeuverPath(value) => Some((
                    EntityKind::ManeuverPath,
                    value.header.source_address.owner_local_keys(),
                )),
                TypedAstDeclaration::ManeuverGate(value) => Some((
                    EntityKind::ManeuverGate,
                    value.header.source_address.owner_local_keys(),
                )),
                TypedAstDeclaration::WaitingZone(value) => Some((
                    EntityKind::WaitingZone,
                    value.header.source_address.owner_local_keys(),
                )),
                _ => None,
            })
            .map(|(kind, owners)| (kind, owners.iter().map(AsRef::as_ref).collect::<Vec<_>>()))
            .collect::<Vec<_>>();

        assert!(owners.contains(&(EntityKind::LaneGroup, vec!["corridor", "section"])));
        assert!(owners.contains(&(EntityKind::FacilityBand, vec!["corridor"])));
        assert!(owners.contains(&(EntityKind::Movement, vec!["junction"])));
        assert!(owners.contains(&(EntityKind::ManeuverPath, vec!["junction", "movement"])));
        assert!(owners.contains(&(
            EntityKind::ManeuverGate,
            vec!["junction", "movement", "path"]
        )));
        assert!(owners.contains(&(
            EntityKind::WaitingZone,
            vec!["junction", "movement", "path"]
        )));
        let junction = declarations
            .iter()
            .find_map(|declaration| match declaration {
                TypedAstDeclaration::Junction(value) => Some(value),
                _ => None,
            })
            .unwrap();
        assert_eq!(junction.approach_edges.len(), 2);
        assert_eq!(
            junction.approach_edges[0].declaration_key().as_ref(),
            "edge-a"
        );
        assert_eq!(
            junction.approach_edges[1].declaration_key().as_ref(),
            "edge-b"
        );
        assert_eq!(junction.internal_edges.len(), 1);
        assert_eq!(
            junction.internal_edges[0].declaration_key().as_ref(),
            "edge-internal"
        );
        let stop_line_namespace = independent
            .iter()
            .find_map(|declaration| match declaration {
                TypedAstDeclaration::StopLine(value) => Some(&value.lane_edge.module_namespace),
                _ => None,
            })
            .unwrap();
        let movement_namespace = declarations
            .iter()
            .find_map(|declaration| match declaration {
                TypedAstDeclaration::Movement(value) => Some(&value.junction.module_namespace),
                _ => None,
            })
            .unwrap();
        assert!(Arc::ptr_eq(stop_line_namespace, movement_namespace));
        assert!(Arc::ptr_eq(stop_line_namespace, &shared_namespace));
    }

    #[test]
    fn junction_relation_locations_preserve_the_owner_canvas_selection() {
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
        for key in ["entry", "internal", "exit"] {
            builder
                .add_declaration(RoadEditingDeclaration::LaneEdge(
                    LaneEdgeInput::try_new(key, 10.0, Vec::new(), None).unwrap(),
                ))
                .unwrap();
        }
        builder
            .add_declaration(RoadEditingDeclaration::Junction(
                JunctionInput::try_new(
                    "junction",
                    vec![
                        LaneEdgeReference::local("entry").unwrap(),
                        LaneEdgeReference::local("exit").unwrap(),
                    ],
                    vec![LaneEdgeReference::local("internal").unwrap()],
                )
                .unwrap()
                .with_canvas_selection("canvas/junction")
                .unwrap(),
            ))
            .unwrap();
        builder
            .add_declaration(RoadEditingDeclaration::Movement(
                MovementInput::try_new(
                    "movement",
                    JunctionReference::local("junction").unwrap(),
                    "entry",
                    "exit",
                )
                .unwrap(),
            ))
            .unwrap();
        builder
            .add_declaration(RoadEditingDeclaration::ManeuverPath(
                ManeuverPathInput::try_new(
                    "path",
                    MovementReference::owner_scoped(vec!["junction".into()], "movement").unwrap(),
                    LaneEdgeReference::local("entry").unwrap(),
                    vec![LaneEdgeReference::local("internal").unwrap()],
                    LaneEdgeReference::local("exit").unwrap(),
                )
                .unwrap()
                .with_canvas_selection("canvas/path")
                .unwrap(),
            ))
            .unwrap();
        let bytes = RoadEditingSourceWriter::new(&limits)
            .write(builder.finish().unwrap())
            .unwrap();
        let input = RoadEditingModuleInput::try_new("roads/main", bytes.as_bytes(), None).unwrap();
        let verified = super::super::reader::verify_source(input, &limits, 0, 0).unwrap();
        let locations = RoadEditingLocationFactory::from_verified_root(verified.root());
        let shared_namespace = Arc::from(verified.root().module_header().authoring_namespace_id());
        let mut declarations = Vec::new();
        lower_owner_scoped_declarations(
            verified.root(),
            &locations,
            &shared_namespace,
            &mut declarations,
        );
        let junction = declarations
            .iter()
            .find_map(|declaration| match declaration {
                TypedAstDeclaration::Junction(value) => Some(value),
                _ => None,
            })
            .unwrap();

        for relation in junction
            .approach_edges
            .iter()
            .chain(junction.internal_edges.iter())
        {
            assert_eq!(
                relation.span.road_editing().unwrap().canvas_selection(),
                Some("canvas/junction")
            );
        }
        let path = declarations
            .iter()
            .find_map(|declaration| match declaration {
                TypedAstDeclaration::ManeuverPath(value) => Some(value),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            path.internal_edges[0]
                .span
                .road_editing()
                .unwrap()
                .canvas_selection(),
            Some("canvas/path")
        );
    }

    #[test]
    fn authoring_topology_and_curves_lower_without_losing_owner_or_parameter_authority() {
        let limits = CompileLimits::p100_initial_v2();
        let module = super::super::writer::tests::module_with_every_declaration(&limits);
        let bytes = RoadEditingSourceWriter::new(&limits).write(module).unwrap();
        let input =
            RoadEditingModuleInput::try_new("road-editing", bytes.as_bytes(), None).unwrap();
        let verified = super::super::reader::verify_source(input, &limits, 0, 0).unwrap();
        let locations = RoadEditingLocationFactory::from_verified_root(verified.root());
        let shared_namespace = Arc::from(verified.root().module_header().authoring_namespace_id());

        let alignments = lower_road_alignments(verified.root(), &locations, &shared_namespace);
        assert_eq!(alignments.len(), 1);
        assert_eq!(alignments[0].road_alignment_key.as_ref(), "alignment");
        assert_eq!(
            alignments[0].canonical_frame.declaration_key().as_ref(),
            "frame"
        );
        assert_eq!(alignments[0].reference_line.start.x, 0.0);
        assert_eq!(alignments[0].reference_line.segments.len(), 1);
        assert!(matches!(
            alignments[0].reference_line.segments[0].geometry,
            AuthoringCurveSegmentGeometry::Line {
                end: AuthoringPoint3F64 { x: 10.0, .. }
            }
        ));
        assert_eq!(
            alignments[0].reference_line.segments[0]
                .span
                .road_editing()
                .unwrap()
                .canvas_selection(),
            Some("canvas/alignment-segment")
        );

        let mut declarations = Vec::new();
        lower_topology_authoring_declarations(
            verified.root(),
            &locations,
            &shared_namespace,
            &mut declarations,
        )
        .unwrap();
        assert_eq!(declarations.len(), 5);
        let section = declarations
            .iter()
            .find_map(|declaration| match declaration {
                TypedAstDeclaration::RoadSection(value) => Some(value),
                _ => None,
            })
            .unwrap();
        assert_eq!(section.lanes.len(), 1);
        let lane = &section.lanes[0];
        assert_eq!(
            lane.header
                .source_address
                .owner_local_keys()
                .iter()
                .map(AsRef::as_ref)
                .collect::<Vec<_>>(),
            ["corridor", "section"]
        );
        assert_eq!(lane.edge_chain[0].declaration_key().as_ref(), "edge-a");
        let lane_geometry = lane.authoring_geometry.as_ref().unwrap();
        assert_eq!(lane_geometry.direction, AuthoringLaneDirection::Forward);
        assert_eq!(lane_geometry.width_profile.start_width_meters, 3.5);

        let internal_edge = declarations
            .iter()
            .find_map(|declaration| match declaration {
                TypedAstDeclaration::LaneEdge(value)
                    if value.header.stable_key.as_ref() == "edge-internal" =>
                {
                    Some(value)
                }
                _ => None,
            })
            .unwrap();
        assert!(matches!(
            internal_edge.geometry_authority,
            LaneEdgeGeometryAuthority::Authoring {
                explicit_curve: Some(_)
            }
        ));
    }

    #[test]
    fn authoring_lane_owner_mismatch_is_a_diagnostic_instead_of_a_lowering_panic() {
        let limits = CompileLimits::p100_initial_v2();
        let module = super::super::writer::tests::module_with_every_declaration(&limits);
        let bytes = RoadEditingSourceWriter::new(&limits).write(module).unwrap();
        let mut malformed = bytes.as_bytes().to_vec();
        let needle = b"corridor>section>lane";
        let replacement = b"corridor>section>fake";
        let positions = malformed
            .windows(needle.len())
            .enumerate()
            .filter_map(|(index, window)| (window == needle).then_some(index))
            .collect::<Vec<_>>();
        assert!(positions.len() >= 2);
        for position in positions {
            malformed[position..position + replacement.len()].copy_from_slice(replacement);
        }

        let input = RoadEditingModuleInput::try_new("road-editing", &malformed, None).unwrap();
        let verified = super::super::reader::verify_source(input, &limits, 0, 0).unwrap();
        let locations = RoadEditingLocationFactory::from_verified_root(verified.root());
        let shared_namespace = Arc::from(verified.root().module_header().authoring_namespace_id());
        let mut declarations = Vec::new();
        assert!(
            lower_topology_authoring_declarations(
                verified.root(),
                &locations,
                &shared_namespace,
                &mut declarations,
            )
            .is_err()
        );
    }

    #[test]
    fn aggregate_declarations_preserve_product_and_set_relation_locations() {
        let limits = CompileLimits::p100_initial_v2();
        let module = super::super::writer::tests::module_with_every_declaration(&limits);
        let bytes = RoadEditingSourceWriter::new(&limits).write(module).unwrap();
        let input =
            RoadEditingModuleInput::try_new("road-editing", bytes.as_bytes(), None).unwrap();
        let verified = super::super::reader::verify_source(input, &limits, 0, 0).unwrap();
        let locations = RoadEditingLocationFactory::from_verified_root(verified.root());
        let shared_namespace = Arc::from(verified.root().module_header().authoring_namespace_id());
        let mut declarations = Vec::new();
        lower_aggregate_declarations(
            verified.root(),
            &locations,
            &shared_namespace,
            &mut declarations,
        );

        assert_eq!(declarations.len(), 3);
        let controller = declarations
            .iter()
            .find_map(|declaration| match declaration {
                TypedAstDeclaration::SignalController(value) => Some(value),
                _ => None,
            })
            .unwrap();
        assert_eq!(controller.signal_groups.len(), 1);
        assert_eq!(controller.phases.len(), 1);
        let phase = &controller.phases[0];
        assert_eq!(
            phase
                .header
                .source_address
                .owner_local_keys()
                .iter()
                .map(AsRef::as_ref)
                .collect::<Vec<_>>(),
            ["controller"]
        );
        assert!(matches!(
            phase
                .controller_relation_span
                .road_editing()
                .unwrap()
                .subject(),
            crate::RoadEditingSubject::OwnerLocal {
                relation: RoadEditingRelationKind::SignalControllerPhase,
                occurrence: RoadEditingRelationOccurrence::OrderedProductOrdinal(0),
                ..
            }
        ));
        assert!(matches!(
            phase.states[0]
                .signal_group
                .span
                .road_editing()
                .unwrap()
                .subject(),
            crate::RoadEditingSubject::OwnerLocal {
                relation: RoadEditingRelationKind::SignalPhaseState,
                occurrence: RoadEditingRelationOccurrence::CanonicalSetOrdinal(0),
                ..
            }
        ));

        let parking = declarations
            .iter()
            .find_map(|declaration| match declaration {
                TypedAstDeclaration::ParkingSpace(value) => Some(value),
                _ => None,
            })
            .unwrap();
        assert_eq!(parking.entry.progress_meters, 1.0);
        assert_eq!(parking.geometry.length_meters, 5.0);
        assert_eq!(
            parking
                .entry
                .lane_edge
                .span
                .road_editing()
                .unwrap()
                .property_path()
                .unwrap()
                .steps()
                .len(),
            2
        );

        let access = declarations
            .iter()
            .find_map(|declaration| match declaration {
                TypedAstDeclaration::AccessRule(value) => Some(value),
                _ => None,
            })
            .unwrap();
        assert_eq!(access.effect, AccessEffect::Allow);
        assert_eq!(access.participant_classes.len(), 1);
        assert!(matches!(
            access.participant_classes[0]
                .span
                .road_editing()
                .unwrap()
                .subject(),
            crate::RoadEditingSubject::OwnerLocal {
                relation: RoadEditingRelationKind::AccessRuleParticipantClass,
                occurrence: RoadEditingRelationOccurrence::CanonicalSetOrdinal(0),
                ..
            }
        ));
    }
}
