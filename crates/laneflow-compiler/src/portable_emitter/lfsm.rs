use super::relations::canonical_relation_tuples;
use super::*;

fn road_editing_relation_code(value: crate::RoadEditingRelationKind) -> u8 {
    match value {
        crate::RoadEditingRelationKind::Import => 0,
        crate::RoadEditingRelationKind::CurveSegment => 1,
        crate::RoadEditingRelationKind::CorridorElement => 2,
        crate::RoadEditingRelationKind::RoadSectionAuthoringLane => 3,
        crate::RoadEditingRelationKind::LaneEdgeSuccessor => 4,
        crate::RoadEditingRelationKind::JunctionApproachEdge => 5,
        crate::RoadEditingRelationKind::JunctionInternalEdge => 6,
        crate::RoadEditingRelationKind::ManeuverPathInternalEdge => 7,
        crate::RoadEditingRelationKind::SignalControllerGroup => 8,
        crate::RoadEditingRelationKind::SignalControllerPhase => 9,
        crate::RoadEditingRelationKind::SignalPhaseState => 10,
        crate::RoadEditingRelationKind::AccessRuleParticipantClass => 11,
    }
}

fn source_language_code(value: crate::SourceLanguage) -> u16 {
    match value {
        crate::SourceLanguage::SyntheticDsl => 1,
        crate::SourceLanguage::RoadEditingSource => 3,
    }
}

fn road_editing_table_code(value: crate::RoadEditingTableKind) -> u16 {
    match value {
        crate::RoadEditingTableKind::RoadEditingSource => 0,
        crate::RoadEditingTableKind::ModuleHeader => 1,
        crate::RoadEditingTableKind::Provenance => 2,
        crate::RoadEditingTableKind::LineSegment => 3,
        crate::RoadEditingTableKind::CubicBezierSegment => 4,
        crate::RoadEditingTableKind::CurveSegment => 5,
        crate::RoadEditingTableKind::CurveProgram => 6,
        crate::RoadEditingTableKind::RoadAlignment => 7,
        crate::RoadEditingTableKind::CorridorElement => 8,
        crate::RoadEditingTableKind::RoadCorridor => 9,
        crate::RoadEditingTableKind::RoadSection => 10,
        crate::RoadEditingTableKind::AuthoringLane => 11,
        crate::RoadEditingTableKind::LaneEdge => 12,
        crate::RoadEditingTableKind::Junction => 13,
        crate::RoadEditingTableKind::Movement => 14,
        crate::RoadEditingTableKind::ManeuverPath => 15,
        crate::RoadEditingTableKind::ManeuverGate => 16,
        crate::RoadEditingTableKind::WaitingZone => 17,
        crate::RoadEditingTableKind::StopLine => 18,
        crate::RoadEditingTableKind::SignalGroup => 19,
        crate::RoadEditingTableKind::SignalController => 20,
        crate::RoadEditingTableKind::SignalPhaseState => 21,
        crate::RoadEditingTableKind::SignalPhase => 22,
        crate::RoadEditingTableKind::ParkingArea => 23,
        crate::RoadEditingTableKind::ParkingLaneAnchor => 24,
        crate::RoadEditingTableKind::ParkingSpaceGeometry => 25,
        crate::RoadEditingTableKind::ParkingSpace => 26,
        crate::RoadEditingTableKind::LaneGroup => 27,
        crate::RoadEditingTableKind::FacilityBand => 28,
        crate::RoadEditingTableKind::ParticipantClass => 29,
        crate::RoadEditingTableKind::AccessRegulation => 30,
        crate::RoadEditingTableKind::AccessRule => 31,
        crate::RoadEditingTableKind::IidmVehicleProfile => 32,
        crate::RoadEditingTableKind::VehicleProfile => 33,
        crate::RoadEditingTableKind::CanonicalFrame => 35,
    }
}

fn road_editing_struct_code(value: crate::RoadEditingStructKind) -> u16 {
    match value {
        crate::RoadEditingStructKind::Digest256 => 0,
        crate::RoadEditingStructKind::OptionalU64 => 1,
        crate::RoadEditingStructKind::Vec3F64 => 2,
        crate::RoadEditingStructKind::LinearWidthProfile => 3,
    }
}

fn road_editing_union_code(value: crate::RoadEditingUnionKind) -> u16 {
    match value {
        crate::RoadEditingUnionKind::CurveSegmentGeometry => 0,
    }
}

fn property_steps(path: Option<&crate::RoadEditingPropertyPath>) -> Option<Box<[(u8, u16, u16)]>> {
    path.map(|path| {
        path.steps()
            .iter()
            .map(|step| match *step {
                crate::RoadEditingPropertyStep::TableField { table, field_id } => {
                    (0, road_editing_table_code(table), field_id)
                }
                crate::RoadEditingPropertyStep::StructMember {
                    structure,
                    member_id,
                } => (1, road_editing_struct_code(structure), u16::from(member_id)),
                crate::RoadEditingPropertyStep::UnionVariant {
                    union,
                    discriminant,
                } => (2, road_editing_union_code(union), u16::from(discriminant)),
            })
            .collect()
    })
}

type DocumentOrdinals<'a> = BTreeMap<&'a str, (u32, u32)>;

fn address_projection(
    address: crate::RoadEditingSourceAddress,
    context: &crate::RoadEditingLocationContext,
) -> RoadEditingAddressProjection {
    let mut owner_local_keys = [None, None, None];
    for (slot, key) in owner_local_keys
        .iter_mut()
        .zip(address.owner_local_keys(context))
    {
        *slot = Some(Box::from(key));
    }
    (
        Box::from(address.module_namespace(context)),
        address.entity_kind().map(EntityKind::code),
        owner_local_keys,
        Box::from(address.local_key(context)),
    )
}

fn location_value<'a>(
    view: crate::SourceLocationView<'a>,
    documents: &DocumentOrdinals<'a>,
) -> Result<LocationValue, PortableEmissionError> {
    let (source_module_ordinal, source_document_ordinal) = documents
        .get(view.source_document_key())
        .copied()
        .ok_or(PortableEmissionError::InternalBindingMismatch)?;
    match view {
        crate::SourceLocationView::Text { start, end, .. } => Ok(LocationValue::Text {
            source_module_ordinal,
            source_document_ordinal,
            start_line: start.line(),
            start_column: start.column(),
            end_line: end.line(),
            end_column: end.column(),
        }),
        crate::SourceLocationView::RoadEditing(location) => {
            if location.byte_range().is_some() {
                return Err(PortableEmissionError::InternalBindingMismatch);
            }
            let context = location.context();
            let mut module_namespace = None;
            let mut entity_kind = None;
            let mut owner_local_keys = [None, None, None];
            let mut local_key = None;
            let mut owner_kind = None;
            let mut relation_kind = None;
            let mut occurrence_kind = None;
            let mut occurrence_ordinal = None;
            let subject_kind = match *location.subject() {
                crate::RoadEditingSubject::ModuleHeader => 0,
                crate::RoadEditingSubject::RoadAlignment { address } => {
                    let (namespace, entity, owners, key) = address_projection(address, context);
                    module_namespace = Some(namespace);
                    entity_kind = entity;
                    owner_local_keys = owners;
                    local_key = Some(key);
                    1
                }
                crate::RoadEditingSubject::Declaration { address } => {
                    let (namespace, entity, owners, key) = address_projection(address, context);
                    module_namespace = Some(namespace);
                    entity_kind = entity;
                    owner_local_keys = owners;
                    local_key = Some(key);
                    2
                }
                crate::RoadEditingSubject::OwnerLocal {
                    owner,
                    relation,
                    occurrence,
                } => {
                    match owner {
                        crate::RoadEditingOwner::ModuleHeader => owner_kind = Some(0),
                        crate::RoadEditingOwner::Address(address) => {
                            owner_kind = Some(1);
                            let (namespace, entity, owners, key) =
                                address_projection(address, context);
                            module_namespace = Some(namespace);
                            entity_kind = entity;
                            owner_local_keys = owners;
                            local_key = Some(key);
                        }
                    }
                    relation_kind = Some(road_editing_relation_code(relation));
                    let (kind, ordinal) = match occurrence {
                        crate::RoadEditingRelationOccurrence::OrderedProductOrdinal(ordinal) => {
                            (0, ordinal)
                        }
                        crate::RoadEditingRelationOccurrence::CanonicalSetOrdinal(ordinal) => {
                            (1, ordinal)
                        }
                    };
                    occurrence_kind = Some(kind);
                    occurrence_ordinal = Some(ordinal);
                    3
                }
                crate::RoadEditingSubject::Wire { .. } => {
                    return Err(PortableEmissionError::InternalBindingMismatch);
                }
            };
            Ok(LocationValue::RoadEditing {
                source_module_ordinal,
                source_document_ordinal,
                subject_kind,
                module_namespace,
                entity_kind,
                owner_local_keys,
                local_key,
                owner_kind,
                relation_kind,
                occurrence_kind,
                occurrence_ordinal,
                property_steps: property_steps(location.property_path()),
                canvas_selection: location.canvas_selection().map(Box::from),
            })
        }
    }
}

fn location_ordinal(
    locations: &[LocationValue],
    location: &LocationValue,
) -> Result<u32, PortableEmissionError> {
    let index = locations
        .binary_search(location)
        .map_err(|_| PortableEmissionError::InternalBindingMismatch)?;
    u32::try_from(index).map_err(|_| PortableEmissionError::ArithmeticOverflow)
}

fn location_set_ordinals(
    locations: &[LocationValue],
    values: &[LocationValue],
) -> Result<Box<[u32]>, PortableEmissionError> {
    let mut values = values.to_vec();
    values.sort_unstable();
    values.dedup();
    values
        .iter()
        .map(|value| location_ordinal(locations, value))
        .collect()
}

fn location_row(ordinal: u32, location: &LocationValue) -> OwnedRow {
    match location {
        LocationValue::Text {
            source_module_ordinal,
            source_document_ordinal,
            start_line,
            start_column,
            end_line,
            end_column,
        } => row([
            field(1, OwnedValue::U32(ordinal)),
            field(2, OwnedValue::U8(0)),
            field(3, OwnedValue::U32(*source_module_ordinal)),
            field(4, OwnedValue::U32(*source_document_ordinal)),
            field(5, OwnedValue::U32(*start_line)),
            field(6, OwnedValue::U32(*start_column)),
            field(7, OwnedValue::U32(*end_line)),
            field(8, OwnedValue::U32(*end_column)),
        ]),
        LocationValue::RoadEditing {
            source_module_ordinal,
            source_document_ordinal,
            subject_kind,
            module_namespace,
            entity_kind,
            owner_local_keys,
            local_key,
            owner_kind,
            relation_kind,
            occurrence_kind,
            occurrence_ordinal,
            property_steps,
            canvas_selection,
        } => {
            let mut fields = vec![
                field(1, OwnedValue::U32(ordinal)),
                field(2, OwnedValue::U8(1)),
                field(3, OwnedValue::U32(*source_module_ordinal)),
                field(4, OwnedValue::U32(*source_document_ordinal)),
                field(9, OwnedValue::U8(*subject_kind)),
            ];
            if let Some(value) = module_namespace {
                fields.push(field(10, OwnedValue::Utf8(value.clone())));
            }
            if let Some(value) = entity_kind {
                fields.push(field(11, OwnedValue::U16(*value)));
            }
            for (tag, value) in (12..=14).zip(owner_local_keys) {
                if let Some(value) = value {
                    fields.push(field(tag, OwnedValue::Utf8(value.clone())));
                }
            }
            if let Some(value) = local_key {
                fields.push(field(15, OwnedValue::Utf8(value.clone())));
            }
            if let Some(value) = owner_kind {
                fields.push(field(16, OwnedValue::U8(*value)));
            }
            if let Some(value) = relation_kind {
                fields.push(field(17, OwnedValue::U8(*value)));
            }
            if let Some(value) = occurrence_kind {
                fields.push(field(18, OwnedValue::U8(*value)));
            }
            if let Some(value) = occurrence_ordinal {
                fields.push(field(19, OwnedValue::U32(*value)));
            }
            if let Some(steps) = property_steps {
                let rows = steps
                    .iter()
                    .map(|(kind, container, member)| {
                        row([
                            field(1, OwnedValue::U8(*kind)),
                            field(2, OwnedValue::U16(*container)),
                            field(3, OwnedValue::U16(*member)),
                        ])
                    })
                    .collect();
                fields.push(field(20, OwnedValue::RecordVector(rows)));
            }
            if let Some(value) = canvas_selection {
                fields.push(field(21, OwnedValue::Utf8(value.clone())));
            }
            row(fields)
        }
    }
}

fn expected_stable_source_keys(lir: &crate::lir::LirUnit) -> Vec<(EntityKind, [u8; 16], u32)> {
    let mut keys = Vec::new();
    macro_rules! append {
        ($kind:expr, $records:expr) => {
            keys.extend($records.iter().map(|record| {
                (
                    $kind,
                    stable_id_bytes(record.stable_id),
                    record.ordinal.raw(),
                )
            }));
        };
    }
    append!(EntityKind::RoadCorridor, lir.road_corridors);
    append!(EntityKind::RoadSection, lir.road_sections);
    append!(EntityKind::AuthoringLane, lir.authoring_lanes);
    append!(EntityKind::LaneEdge, lir.lane_edges);
    append!(EntityKind::Junction, lir.junctions);
    append!(EntityKind::Movement, lir.movements);
    append!(EntityKind::ManeuverPath, lir.maneuver_paths);
    append!(EntityKind::ManeuverGate, lir.maneuver_gates);
    append!(EntityKind::WaitingZone, lir.waiting_zones);
    append!(EntityKind::StopLine, lir.stop_lines);
    append!(EntityKind::SignalGroup, lir.signal_groups);
    append!(EntityKind::SignalController, lir.signal_controllers);
    append!(EntityKind::SignalPhase, lir.signal_phases);
    append!(EntityKind::ParkingArea, lir.parking_areas);
    append!(EntityKind::ParkingSpace, lir.parking_spaces);
    append!(EntityKind::LaneGroup, lir.lane_groups);
    append!(EntityKind::FacilityBand, lir.facility_bands);
    append!(EntityKind::ParticipantClass, lir.participant_classes);
    append!(EntityKind::AccessRule, lir.access_rules);
    append!(EntityKind::VehicleProfile, lir.vehicle_profiles);
    append!(EntityKind::CanonicalFrame, lir.canonical_frames);
    keys.sort_unstable();
    keys
}

fn expected_owner_local_source_keys(
    lir: &crate::lir::LirUnit,
) -> Vec<(EntityKind, [u8; 16], u8, u32)> {
    let mut keys: Vec<_> = canonical_relation_tuples(lir)
        .into_iter()
        .map(|relation| {
            (
                relation.owner_entity_kind,
                relation.owner_stable_id,
                relation.role,
                relation.local_index,
            )
        })
        .collect();
    for phase in &lir.signal_phases {
        for (local_index, _) in lir.signal_phase_states[phase.states.as_usize_range()]
            .iter()
            .enumerate()
        {
            keys.push((
                EntityKind::SignalPhase,
                stable_id_bytes(phase.stable_id),
                19,
                u32::try_from(local_index).expect("compile limits cap relation counts at u32"),
            ));
        }
    }
    let mut next_lane_geometry_index = vec![0_u32; lir.canonical_frames.len()];
    for geometry in &lir.lane_edge_geometries {
        let frame = geometry.canonical_frame;
        let local_index = next_lane_geometry_index[frame.index()];
        next_lane_geometry_index[frame.index()] += 1;
        keys.push((
            EntityKind::CanonicalFrame,
            stable_id_bytes(lir.canonical_frames[frame.index()].stable_id),
            28,
            local_index,
        ));
    }
    let mut next_facility_geometry_index = vec![0_u32; lir.canonical_frames.len()];
    for geometry in &lir.facility_band_geometries {
        let frame = geometry.canonical_frame;
        let local_index = next_facility_geometry_index[frame.index()];
        next_facility_geometry_index[frame.index()] += 1;
        keys.push((
            EntityKind::CanonicalFrame,
            stable_id_bytes(lir.canonical_frames[frame.index()].stable_id),
            29,
            local_index,
        ));
    }
    keys.sort_unstable();
    keys
}

pub(super) fn build_lfsm(
    output: &CompilationOutput,
    provenance: &PortableEmissionProvenance,
    source_collection_digest: [u8; 32],
    network_revision: NetworkRevisionId,
    artifact: &PortableObjectCandidate,
) -> Result<OwnedObject, PortableEmissionError> {
    let source_map = output.source_map_input();
    let modules: Vec<_> = source_map.source_modules().collect();
    let module_ordinals: BTreeMap<_, _> = modules
        .iter()
        .enumerate()
        .map(|(ordinal, module)| {
            Ok((
                module.authoring_namespace_id(),
                u32::try_from(ordinal).map_err(|_| PortableEmissionError::ArithmeticOverflow)?,
            ))
        })
        .collect::<Result<_, PortableEmissionError>>()?;
    let mut documents: Vec<_> = source_map
        .source_documents()
        .map(|document| {
            let module = module_ordinals
                .get(document.authoring_namespace_id())
                .copied()
                .ok_or(PortableEmissionError::InternalBindingMismatch)?;
            Ok((module, document.source_document_key(), document))
        })
        .collect::<Result<_, PortableEmissionError>>()?;
    documents.sort_unstable_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.as_bytes().cmp(right.1.as_bytes()))
    });
    let document_ordinals: DocumentOrdinals<'_> = documents
        .iter()
        .enumerate()
        .map(|(ordinal, (module, key, _))| {
            Ok((
                *key,
                (
                    *module,
                    u32::try_from(ordinal)
                        .map_err(|_| PortableEmissionError::ArithmeticOverflow)?,
                ),
            ))
        })
        .collect::<Result<_, PortableEmissionError>>()?;
    let lir = output.lir().unit();

    let mut stable_sources = Vec::new();
    macro_rules! append_stable_sources {
        ($kind:expr, $iterator:expr) => {
            for source in $iterator {
                stable_sources.push(StableSourceProjection {
                    entity_kind: $kind,
                    stable_id: stable_id_bytes(source.stable_id()),
                    typed_ordinal: source.ordinal().raw(),
                    primary: location_value(source.primary_source(), &document_ordinals)?,
                    contributing: source
                        .contributing_sources()
                        .map(|location| location_value(location, &document_ordinals))
                        .collect::<Result<_, _>>()?,
                });
            }
        };
    }
    append_stable_sources!(EntityKind::RoadCorridor, source_map.road_corridor_sources());
    append_stable_sources!(EntityKind::RoadSection, source_map.road_section_sources());
    append_stable_sources!(
        EntityKind::AuthoringLane,
        source_map.authoring_lane_sources()
    );
    append_stable_sources!(EntityKind::LaneEdge, source_map.lane_edge_sources());
    append_stable_sources!(EntityKind::Junction, source_map.junction_sources());
    append_stable_sources!(EntityKind::Movement, source_map.movement_sources());
    append_stable_sources!(EntityKind::ManeuverPath, source_map.maneuver_path_sources());
    append_stable_sources!(EntityKind::ManeuverGate, source_map.maneuver_gate_sources());
    append_stable_sources!(EntityKind::WaitingZone, source_map.waiting_zone_sources());
    append_stable_sources!(EntityKind::StopLine, source_map.stop_line_sources());
    append_stable_sources!(EntityKind::SignalGroup, source_map.signal_group_sources());
    append_stable_sources!(
        EntityKind::SignalController,
        source_map.signal_controller_sources()
    );
    append_stable_sources!(EntityKind::SignalPhase, source_map.signal_phase_sources());
    append_stable_sources!(EntityKind::ParkingArea, source_map.parking_area_sources());
    append_stable_sources!(EntityKind::ParkingSpace, source_map.parking_space_sources());
    append_stable_sources!(EntityKind::LaneGroup, source_map.lane_group_sources());
    append_stable_sources!(EntityKind::FacilityBand, source_map.facility_band_sources());
    append_stable_sources!(
        EntityKind::ParticipantClass,
        source_map.participant_class_sources()
    );
    append_stable_sources!(EntityKind::AccessRule, source_map.access_rule_sources());
    append_stable_sources!(
        EntityKind::VehicleProfile,
        source_map.vehicle_profile_sources()
    );
    append_stable_sources!(
        EntityKind::CanonicalFrame,
        source_map.canonical_frame_sources()
    );
    stable_sources.sort_unstable_by_key(|source| {
        (source.entity_kind, source.stable_id, source.typed_ordinal)
    });
    let actual_stable_keys: Vec<_> = stable_sources
        .iter()
        .map(|source| (source.entity_kind, source.stable_id, source.typed_ordinal))
        .collect();
    if actual_stable_keys != expected_stable_source_keys(lir) {
        return Err(PortableEmissionError::InternalBindingMismatch);
    }

    let mut owner_local_sources = Vec::new();
    let mut spatial_ranges = Vec::new();
    let internal_edges: Vec<bool> = (0..lir.lane_edges.len())
        .map(|ordinal| {
            lir.junction_internal_edges
                .binary_search_by_key(
                    &u32::try_from(ordinal).expect("compile limits cap entity counts at u32"),
                    |entry| entry.edge.raw(),
                )
                .is_ok()
        })
        .collect();
    let mut next_successor_index_by_owner = vec![0_u32; lir.lane_edges.len()];
    for source in source_map.lane_edge_successor_sources() {
        let owner = source.owner_ordinal();
        let edge = &lir.lane_edges[owner.index()];
        let target = lir.lane_edge_successors[edge.successors.as_usize_range()][usize::try_from(
            source.local_index(),
        )
        .expect("supported compiler targets can index a validated local relation")];
        if internal_edges[owner.index()] || internal_edges[target.index()] {
            continue;
        }
        let local_index = next_successor_index_by_owner[owner.index()];
        next_successor_index_by_owner[owner.index()] += 1;
        owner_local_sources.push(OwnerLocalProjection {
            owner_entity_kind: EntityKind::LaneEdge,
            owner_stable_id: stable_id_bytes(source.owner_stable_id()),
            role: source_relation_role_code(source.role()),
            local_index,
            primary: location_value(source.primary_source(), &document_ordinals)?,
            contributing: source
                .contributing_sources()
                .map(|location| location_value(location, &document_ordinals))
                .collect::<Result<_, _>>()?,
        });
    }

    macro_rules! push_owner_local {
        ($kind:expr, $stable_id:expr, $source:expr) => {{
            let source = $source;
            owner_local_sources.push(OwnerLocalProjection {
                owner_entity_kind: $kind,
                owner_stable_id: stable_id_bytes($stable_id),
                role: source_relation_role_code(source.role()),
                local_index: source.local_index(),
                primary: location_value(source.primary_source(), &document_ordinals)?,
                contributing: source
                    .contributing_sources()
                    .map(|location| location_value(location, &document_ordinals))
                    .collect::<Result<_, _>>()?,
            });
        }};
    }
    for source in source_map.cross_section_relation_sources() {
        match source.owner() {
            crate::CrossSectionRelationOwner::RoadCorridor(_, id) => {
                push_owner_local!(EntityKind::RoadCorridor, id, source)
            }
            crate::CrossSectionRelationOwner::RoadSection(_, id) => {
                push_owner_local!(EntityKind::RoadSection, id, source)
            }
            crate::CrossSectionRelationOwner::AuthoringLane(_, id) => {
                push_owner_local!(EntityKind::AuthoringLane, id, source)
            }
            crate::CrossSectionRelationOwner::LaneGroup(_, id) => {
                push_owner_local!(EntityKind::LaneGroup, id, source)
            }
        }
    }
    for source in source_map.junction_relation_sources() {
        match source.owner() {
            crate::JunctionRelationOwner::Junction(_, id) => {
                push_owner_local!(EntityKind::Junction, id, source)
            }
            crate::JunctionRelationOwner::Movement(_, id) => {
                push_owner_local!(EntityKind::Movement, id, source)
            }
            crate::JunctionRelationOwner::ManeuverPath(_, id) => {
                push_owner_local!(EntityKind::ManeuverPath, id, source)
            }
            crate::JunctionRelationOwner::StopLine(_, id) => {
                push_owner_local!(EntityKind::StopLine, id, source)
            }
        }
    }
    for source in source_map.signal_relation_sources() {
        match source.owner() {
            crate::SignalRelationOwner::SignalController(_, id) => {
                push_owner_local!(EntityKind::SignalController, id, source)
            }
            crate::SignalRelationOwner::SignalPhase(_, id) => {
                push_owner_local!(EntityKind::SignalPhase, id, source)
            }
            crate::SignalRelationOwner::ManeuverGate(_, id) => {
                push_owner_local!(EntityKind::ManeuverGate, id, source)
            }
        }
    }
    for source in source_map.parking_relation_sources() {
        push_owner_local!(EntityKind::ParkingSpace, source.owner_stable_id(), source);
    }
    for source in source_map.access_relation_sources() {
        match source.owner() {
            crate::AccessRelationOwner::ParticipantClass(_, id) => {
                push_owner_local!(EntityKind::ParticipantClass, id, source)
            }
            crate::AccessRelationOwner::VehicleProfile(_, id) => {
                push_owner_local!(EntityKind::VehicleProfile, id, source)
            }
            crate::AccessRelationOwner::AccessRule(_, id) => {
                push_owner_local!(EntityKind::AccessRule, id, source)
            }
        }
    }
    for source in source_map.spatial_relation_sources() {
        let owner_entity_kind = EntityKind::CanonicalFrame;
        let owner_stable_id = stable_id_bytes(source.owner_stable_id());
        let role = source_relation_role_code(source.role());
        let local_index = source.local_index();
        let primary = location_value(source.primary_source(), &document_ordinals)?;
        let contributing = source
            .contributing_sources()
            .map(|location| location_value(location, &document_ordinals))
            .collect::<Result<Vec<_>, _>>()?;
        for range in source.geometry_source_ranges() {
            let points = range.point_range();
            spatial_ranges.push(SpatialRangeProjection {
                owner_entity_kind,
                owner_stable_id,
                role,
                local_index,
                point_start: points.start,
                point_end_exclusive: points.end,
                source_segment_ordinal: range.source_segment_ordinal(),
                source: location_value(range.source(), &document_ordinals)?,
            });
        }
        owner_local_sources.push(OwnerLocalProjection {
            owner_entity_kind,
            owner_stable_id,
            role,
            local_index,
            primary,
            contributing,
        });
    }
    owner_local_sources.sort_unstable_by_key(|source| {
        (
            source.owner_entity_kind,
            source.owner_stable_id,
            source.role,
            source.local_index,
        )
    });
    let actual_owner_local_keys: Vec<_> = owner_local_sources
        .iter()
        .map(|source| {
            (
                source.owner_entity_kind,
                source.owner_stable_id,
                source.role,
                source.local_index,
            )
        })
        .collect();
    if actual_owner_local_keys != expected_owner_local_source_keys(lir) {
        return Err(PortableEmissionError::InternalBindingMismatch);
    }
    spatial_ranges.sort_unstable_by_key(|source| {
        (
            source.owner_entity_kind,
            source.owner_stable_id,
            source.role,
            source.local_index,
            source.point_start,
        )
    });

    let module_source_views: Vec<_> = source_map.source_module_sources().collect();
    if module_source_views.len() != modules.len() {
        return Err(PortableEmissionError::InternalBindingMismatch);
    }
    let mut locations = Vec::new();
    for source in &module_source_views {
        locations.push(location_value(source.primary_source(), &document_ordinals)?);
    }
    for source in &stable_sources {
        locations.push(source.primary.clone());
        locations.extend(source.contributing.iter().cloned());
    }
    for source in &owner_local_sources {
        locations.push(source.primary.clone());
        locations.extend(source.contributing.iter().cloned());
    }
    locations.extend(spatial_ranges.iter().map(|range| range.source.clone()));
    locations.sort_unstable();
    locations.dedup();

    let source_module_rows = module_source_views
        .iter()
        .enumerate()
        .map(|(ordinal, source)| {
            let descriptor = source.descriptor();
            let mut fields = vec![
                field(
                    1,
                    OwnedValue::U32(
                        u32::try_from(ordinal).expect("compile limits cap source modules at u32"),
                    ),
                ),
                field(
                    2,
                    OwnedValue::Utf8(Box::from(descriptor.authoring_namespace_id())),
                ),
                field(
                    3,
                    OwnedValue::U16(source_language_code(descriptor.source_language())),
                ),
                field(
                    4,
                    OwnedValue::Sha256(*descriptor.source_document_set_digest()),
                ),
                field(
                    5,
                    OwnedValue::U32(descriptor.source_document_set_digest_version()),
                ),
                field(6, OwnedValue::U32(descriptor.frontend_version())),
                field(7, OwnedValue::Sha256(*descriptor.frontend_options_digest())),
                field(
                    8,
                    OwnedValue::Utf8(Box::from(descriptor.generator_build_id())),
                ),
                field(
                    9,
                    OwnedValue::Sha256(*descriptor.parameters_and_inputs_digest()),
                ),
            ];
            if let Some(seed) = descriptor.random_seed() {
                fields.push(field(10, OwnedValue::U64(seed)));
            }
            fields.push(field(
                11,
                OwnedValue::Utf8(Box::from(descriptor.provenance())),
            ));
            let imports = descriptor
                .imports()
                .map(|namespace| row([field(1, OwnedValue::Utf8(Box::from(namespace)))]))
                .collect();
            fields.push(field(12, OwnedValue::RecordVector(imports)));
            let primary = location_value(source.primary_source(), &document_ordinals)?;
            fields.push(field(
                13,
                OwnedValue::U32(location_ordinal(&locations, &primary)?),
            ));
            Ok(row(fields))
        })
        .collect::<Result<Vec<_>, PortableEmissionError>>()?;
    let source_document_rows =
        documents
            .iter()
            .enumerate()
            .map(|(ordinal, (module_ordinal, _, document))| {
                let mut fields = vec![
                    field(
                        1,
                        OwnedValue::U32(
                            u32::try_from(ordinal)
                                .expect("compile limits cap source documents at u32"),
                        ),
                    ),
                    field(2, OwnedValue::U32(*module_ordinal)),
                    field(
                        3,
                        OwnedValue::Utf8(Box::from(document.source_document_key())),
                    ),
                    field(4, OwnedValue::Sha256(*document.source_document_digest())),
                    field(5, OwnedValue::U32(document.source_record_byte_len())),
                ];
                if let Some(display_source) = document.origin().display_source() {
                    fields.push(field(6, OwnedValue::Utf8(Box::from(display_source))));
                }
                row(fields)
            });
    let source_location_rows = locations.iter().enumerate().map(|(ordinal, location)| {
        location_row(
            u32::try_from(ordinal).expect("compile limits cap source locations at u32"),
            location,
        )
    });
    let stable_source_rows = stable_sources
        .iter()
        .map(|source| {
            Ok(row([
                field(1, OwnedValue::U16(source.entity_kind.code())),
                field(2, OwnedValue::StableId128(source.stable_id)),
                field(3, OwnedValue::U32(source.typed_ordinal)),
                field(
                    4,
                    OwnedValue::U32(location_ordinal(&locations, &source.primary)?),
                ),
                field(
                    5,
                    OwnedValue::OrdinalVectorU32(location_set_ordinals(
                        &locations,
                        &source.contributing,
                    )?),
                ),
            ]))
        })
        .collect::<Result<Vec<_>, PortableEmissionError>>()?;
    let owner_local_rows = owner_local_sources
        .iter()
        .map(|source| {
            Ok(row([
                field(1, OwnedValue::U16(source.owner_entity_kind.code())),
                field(2, OwnedValue::StableId128(source.owner_stable_id)),
                field(3, OwnedValue::U8(source.role)),
                field(4, OwnedValue::U32(source.local_index)),
                field(
                    5,
                    OwnedValue::U32(location_ordinal(&locations, &source.primary)?),
                ),
                field(
                    6,
                    OwnedValue::OrdinalVectorU32(location_set_ordinals(
                        &locations,
                        &source.contributing,
                    )?),
                ),
            ]))
        })
        .collect::<Result<Vec<_>, PortableEmissionError>>()?;
    let spatial_range_rows = spatial_ranges
        .iter()
        .map(|source| {
            Ok(row([
                field(1, OwnedValue::U16(source.owner_entity_kind.code())),
                field(2, OwnedValue::StableId128(source.owner_stable_id)),
                field(3, OwnedValue::U8(source.role)),
                field(4, OwnedValue::U32(source.local_index)),
                field(5, OwnedValue::U32(source.point_start)),
                field(6, OwnedValue::U32(source.point_end_exclusive)),
                field(7, OwnedValue::U32(source.source_segment_ordinal)),
                field(
                    8,
                    OwnedValue::U32(location_ordinal(&locations, &source.source)?),
                ),
            ]))
        })
        .collect::<Result<Vec<_>, PortableEmissionError>>()?;
    let derived_rows = owner_local_sources
        .iter()
        .filter(|source| matches!(source.role, 9 | 14 | 15 | 16))
        .map(|source| {
            let mut source_locations = source.contributing.clone();
            source_locations.push(source.primary.clone());
            let constraint_version = if source.role == 9 {
                CONSTRAINT_CONTRACT_VERSION
            } else {
                STATIC_EXECUTION_CONTRACT_VERSION
            };
            Ok(row([
                field(1, OwnedValue::U16(source.owner_entity_kind.code())),
                field(2, OwnedValue::StableId128(source.owner_stable_id)),
                field(3, OwnedValue::U8(source.role)),
                field(4, OwnedValue::U32(source.local_index)),
                field(5, OwnedValue::U16(1)),
                field(6, OwnedValue::U16(constraint_version)),
                field(
                    7,
                    OwnedValue::OrdinalVectorU32(location_set_ordinals(
                        &locations,
                        &source_locations,
                    )?),
                ),
            ]))
        })
        .collect::<Result<Vec<_>, PortableEmissionError>>()?;

    Ok(OwnedObject {
        kind: PortableObjectKind::SourceMap,
        sections: vec![
            section(
                1,
                [table(
                    1,
                    [row([
                        field(1, OwnedValue::U16(NETWORK_REVISION_DERIVATION_VERSION)),
                        field(
                            2,
                            OwnedValue::Sha256(network_revision.into_digest().into_bytes()),
                        ),
                        field(3, OwnedValue::U16(CANONICAL_ARTIFACT_FORMAT_VERSION)),
                        field(4, OwnedValue::Sha256(artifact.digest().into_bytes())),
                        field(5, OwnedValue::U64(artifact.byte_length().get())),
                        field(6, OwnedValue::Utf8(provenance.compiler_build_id.clone())),
                        field(7, OwnedValue::U16(SOURCE_COLLECTION_DIGEST_VERSION_V1)),
                        field(8, OwnedValue::Sha256(source_collection_digest)),
                    ])],
                )],
            ),
            section(
                2,
                [
                    table(1, source_module_rows),
                    table(2, source_document_rows),
                    table(3, source_location_rows),
                ],
            ),
            section(3, [table(1, stable_source_rows)]),
            section(
                4,
                [table(1, owner_local_rows), table(2, spatial_range_rows)],
            ),
            section(5, [table(1, derived_rows)]),
        ]
        .into_boxed_slice(),
    })
}
