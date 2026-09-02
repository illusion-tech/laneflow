use super::*;

pub(super) fn build_lfca(
    output: &CompilationOutput,
    provenance: &PortableEmissionProvenance,
    source_collection_digest: [u8; 32],
    declared_network_revision: NetworkRevisionId,
) -> Result<OwnedObject, PortableEmissionError> {
    let lir = output.lir().unit();
    let direction_profile = lir.geometry_profiles.map_or(0, |profiles| {
        geometry_direction_profile_code(profiles.direction)
    });
    let accuracy_profile = lir.geometry_profiles.map_or(0, |profiles| {
        geometry_accuracy_profile_code(profiles.accuracy)
    });
    let spatial_present = u8::from(
        lir.geometry_profiles.is_some()
            || !lir.canonical_frames.is_empty()
            || !lir.lane_edge_geometries.is_empty()
            || !lir.facility_band_geometries.is_empty()
            || !lir.conflict_zone_regions.is_empty(),
    );

    Ok(OwnedObject {
        kind: PortableObjectKind::CanonicalArtifact,
        sections: vec![
            section(
                1,
                [table(
                    1,
                    [row([
                        field(1, OwnedValue::U16(CANONICAL_ARTIFACT_FORMAT_VERSION)),
                        field(2, OwnedValue::U16(IDENTITY_ENCODING_VERSION)),
                        field(3, OwnedValue::U16(IDENTITY_REGISTRY_REVISION)),
                        field(4, OwnedValue::U16(NETWORK_REVISION_DERIVATION_VERSION)),
                        field(5, OwnedValue::U16(CONSTRAINT_CONTRACT_VERSION)),
                        field(6, OwnedValue::U16(STATIC_EXECUTION_CONTRACT_VERSION)),
                    ])],
                )],
            ),
            section(2, [table(1, canonical_identity_rows(lir))]),
            section(3, canonical_entity_tables(lir)?),
            section(4, canonical_relation_tables(lir)),
            section(
                5,
                [
                    table(
                        1,
                        [row([
                            field(1, OwnedValue::U8(spatial_present)),
                            field(2, OwnedValue::U8(direction_profile)),
                        ])],
                    ),
                    table(2, lane_edge_geometry_rows(output, direction_profile)),
                    table(3, facility_band_geometry_rows(output, direction_profile)),
                    table(4, conflict_zone_region_rows(lir)),
                ],
            ),
            section(
                6,
                [table(
                    1,
                    [row([
                        field(1, OwnedValue::U16(STATIC_EXECUTION_CONTRACT_VERSION)),
                        field(2, OwnedValue::U16(CONSTRAINT_CONTRACT_VERSION)),
                    ])],
                )],
            ),
            section(
                7,
                [table(
                    1,
                    [row([
                        field(1, OwnedValue::Utf8(provenance.compiler_build_id.clone())),
                        field(2, OwnedValue::U16(SOURCE_COLLECTION_DIGEST_VERSION_V1)),
                        field(3, OwnedValue::Sha256(source_collection_digest)),
                        field(4, OwnedValue::Sha256(PORTABLE_COMPILE_OPTIONS_DIGEST_V1)),
                        field(5, OwnedValue::U16(CHUNKED_EMITTER_VERSION)),
                        field(6, OwnedValue::U8(accuracy_profile)),
                    ])],
                )],
            ),
            section(
                8,
                [table(
                    1,
                    [row([field(
                        1,
                        OwnedValue::Sha256(declared_network_revision.into_digest().into_bytes()),
                    )])],
                )],
            ),
        ]
        .into_boxed_slice(),
    })
}

fn ordinals<K: OrdinalKind + Copy>(values: &[Ordinal<K>]) -> Box<[u32]> {
    values.iter().map(|value| value.raw()).collect()
}

fn identity_fields(
    lir: &crate::lir::LirUnit,
    range: crate::arena::TableRange<crate::lir::LirIdentityField>,
) -> OwnedValue {
    let rows = lir.identity_fields[range.as_usize_range()]
        .iter()
        .map(|identity| {
            row([
                field(1, OwnedValue::U16(identity.tag.code())),
                field(
                    2,
                    OwnedValue::Bytes(
                        lir.identity_field_bytes[identity.value_bytes.as_usize_range()]
                            .to_vec()
                            .into_boxed_slice(),
                    ),
                ),
            ])
        })
        .collect();
    OwnedValue::RecordVector(rows)
}

fn canonical_identity_rows(lir: &crate::lir::LirUnit) -> Vec<OwnedRow> {
    let mut rows = Vec::new();
    macro_rules! append {
        ($kind:expr, $records:expr) => {
            rows.extend($records.iter().map(|record| {
                row([
                    field(1, OwnedValue::U16($kind.code())),
                    field(2, OwnedValue::U32(record.ordinal.raw())),
                    field(
                        3,
                        OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
                    ),
                    field(4, identity_fields(lir, record.identity_fields)),
                ])
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
    append!(EntityKind::ParkingFacility, lir.parking_facilities);
    append!(EntityKind::ParkingSpace, lir.parking_spaces);
    append!(EntityKind::LaneGroup, lir.lane_groups);
    append!(EntityKind::FacilityBand, lir.facility_bands);
    append!(EntityKind::ParticipantClass, lir.participant_classes);
    append!(EntityKind::AccessRule, lir.access_rules);
    append!(EntityKind::VehicleProfile, lir.vehicle_profiles);
    append!(EntityKind::ConflictZone, lir.conflict_zones);
    append!(EntityKind::CanonicalFrame, lir.canonical_frames);
    append!(EntityKind::ParticipantStream, lir.participant_streams);
    rows
}

fn canonical_entity_tables(
    lir: &crate::lir::LirUnit,
) -> Result<Vec<OwnedTable>, PortableEmissionError> {
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

    let road_corridors = lir.road_corridors.iter().map(|record| {
        let elements = lir.corridor_elements[record.elements.as_usize_range()]
            .iter()
            .map(|element| match element {
                crate::lir::LirCorridorElement::RoadSection(ordinal) => row([
                    field(1, OwnedValue::U8(0)),
                    field(2, OwnedValue::U32(ordinal.raw())),
                ]),
                crate::lir::LirCorridorElement::FacilityBand(ordinal) => row([
                    field(1, OwnedValue::U8(1)),
                    field(2, OwnedValue::U32(ordinal.raw())),
                ]),
            })
            .collect();
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::U32(record.reference_section.raw())),
            field(4, OwnedValue::RecordVector(elements)),
        ])
    });
    let road_sections = lir.road_sections.iter().map(|record| {
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::U32(record.road_corridor.raw())),
            field(4, OwnedValue::Utf8(record.kind_id.clone())),
            field(
                5,
                OwnedValue::OrdinalVectorU32(ordinals(
                    &lir.road_section_lanes[record.lanes.as_usize_range()],
                )),
            ),
        ])
    });
    let authoring_lanes = lir.authoring_lanes.iter().map(|record| {
        let mut fields = vec![
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::U32(record.road_section.raw())),
            field(
                4,
                OwnedValue::OrdinalVectorU32(ordinals(
                    &lir.authoring_lane_edges[record.edge_chain.as_usize_range()],
                )),
            ),
        ];
        if let Some(group) = record.lane_group {
            fields.push(field(5, OwnedValue::U32(group.raw())));
        }
        row(fields)
    });
    let lane_length_mm = lir
        .lane_edges
        .iter()
        .map(|record| closed_lane_edge_length_mm(lir, record))
        .collect::<Result<Vec<_>, PortableEmissionError>>()?;
    let lane_edges = lir
        .lane_edges
        .iter()
        .map(|record| {
            let successors = lir.lane_edge_successors[record.successors.as_usize_range()]
                .iter()
                .copied()
                .filter(|successor| {
                    !internal_edges[record.ordinal.index()] && !internal_edges[successor.index()]
                })
                .map(|ordinal| ordinal.raw())
                .collect();
            Ok(row([
                field(1, OwnedValue::U32(record.ordinal.raw())),
                field(
                    2,
                    OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
                ),
                field(3, OwnedValue::U32(lane_length_mm[record.ordinal.index()])),
                field(4, OwnedValue::U32(record.speed_limit_mm_s)),
                field(5, OwnedValue::OrdinalVectorU32(successors)),
            ]))
        })
        .collect::<Result<Vec<_>, PortableEmissionError>>()?;
    let junctions = lir.junctions.iter().map(|record| {
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(
                3,
                OwnedValue::OrdinalVectorU32(ordinals(
                    &lir.junction_movements[record.movements.as_usize_range()],
                )),
            ),
        ])
    });
    let movements = lir.movements.iter().map(|record| {
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::U32(record.junction.raw())),
            field(
                4,
                OwnedValue::Utf8(record.directed_entry_approach_key.clone()),
            ),
            field(
                5,
                OwnedValue::Utf8(record.directed_exit_approach_key.clone()),
            ),
            field(
                6,
                OwnedValue::OrdinalVectorU32(ordinals(
                    &lir.movement_maneuver_paths[record.maneuver_paths.as_usize_range()],
                )),
            ),
        ])
    });
    let maneuver_paths = lir.maneuver_paths.iter().map(|record| {
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::U32(record.movement.raw())),
            field(
                4,
                OwnedValue::OrdinalVectorU32(ordinals(
                    &lir.maneuver_path_edges[record.edges.as_usize_range()],
                )),
            ),
            field(
                5,
                OwnedValue::OrdinalVectorU32(ordinals(
                    &lir.maneuver_path_gates[record.maneuver_gates.as_usize_range()],
                )),
            ),
            field(
                6,
                OwnedValue::OrdinalVectorU32(ordinals(
                    &lir.maneuver_path_waiting_zones[record.waiting_zones.as_usize_range()],
                )),
            ),
        ])
    });
    let maneuver_gates = lir.maneuver_gates.iter().map(|record| {
        let mut fields = vec![
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::U32(record.maneuver_path.raw())),
            field(4, OwnedValue::U32(record.transition_index)),
            field(5, OwnedValue::U32(record.stop_line.raw())),
        ];
        match record.signal_control {
            crate::lir::LirSignalControl::None => fields.push(field(6, OwnedValue::U8(0))),
            crate::lir::LirSignalControl::Group(group) => {
                fields.push(field(6, OwnedValue::U8(1)));
                fields.push(field(7, OwnedValue::U32(group.raw())));
            }
        }
        row(fields)
    });
    let waiting_zones = lir.waiting_zones.iter().map(|record| {
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::U32(record.maneuver_path.raw())),
            field(4, OwnedValue::U32(record.entry_gate.raw())),
            field(5, OwnedValue::U32(record.release_gate.raw())),
            field(6, OwnedValue::U32(record.max_occupancy)),
        ])
    });
    let stop_lines = lir.stop_lines.iter().map(|record| {
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::U32(record.lane_edge.raw())),
            field(
                4,
                OwnedValue::OrdinalVectorU32(ordinals(
                    &lir.stop_line_maneuver_gates[record.maneuver_gates.as_usize_range()],
                )),
            ),
        ])
    });
    let signal_groups = lir.signal_groups.iter().map(|record| {
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::U32(record.controller.raw())),
            field(
                4,
                OwnedValue::OrdinalVectorU32(ordinals(
                    &lir.signal_group_maneuver_gates[record.maneuver_gates.as_usize_range()],
                )),
            ),
        ])
    });
    let signal_controllers = lir.signal_controllers.iter().map(|record| {
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::U64(record.offset_ms)),
            field(4, OwnedValue::U64(record.cycle_duration_ms)),
            field(
                5,
                OwnedValue::OrdinalVectorU32(ordinals(
                    &lir.signal_controller_groups[record.signal_groups.as_usize_range()],
                )),
            ),
            field(
                6,
                OwnedValue::OrdinalVectorU32(ordinals(
                    &lir.signal_controller_phases[record.phases.as_usize_range()],
                )),
            ),
        ])
    });
    let signal_phases = lir.signal_phases.iter().map(|record| {
        let states = lir.signal_phase_states[record.states.as_usize_range()]
            .iter()
            .map(|state| {
                let aspect = match state.aspect {
                    laneflow_static_contract::SignalAspect::Red => 0,
                    laneflow_static_contract::SignalAspect::Yellow => 1,
                    laneflow_static_contract::SignalAspect::Green => 2,
                    _ => unreachable!("validated LIR only stores the closed v1 signal aspects"),
                };
                row([
                    field(1, OwnedValue::U32(state.signal_group.raw())),
                    field(2, OwnedValue::U8(aspect)),
                ])
            })
            .collect();
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::U32(record.controller.raw())),
            field(4, OwnedValue::U64(record.duration_ms)),
            field(5, OwnedValue::RecordVector(states)),
        ])
    });
    let parking_facilities = lir.parking_facilities.iter().map(|record| {
        let virtual_entries = lir.parking_facility_virtual_entries
            [record.virtual_entries.as_usize_range()]
        .iter()
        .map(|anchor| {
            row([
                field(1, OwnedValue::U32(anchor.lane_edge.raw())),
                field(2, OwnedValue::U32(anchor.progress_mm)),
            ])
        })
        .collect();
        let virtual_exits = lir.parking_facility_virtual_exits
            [record.virtual_exits.as_usize_range()]
        .iter()
        .map(|anchor| {
            row([
                field(1, OwnedValue::U32(anchor.lane_edge.raw())),
                field(2, OwnedValue::U32(anchor.progress_mm)),
            ])
        })
        .collect();
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(
                3,
                OwnedValue::OrdinalVectorU32(ordinals(
                    &lir.parking_facility_spaces[record.parking_spaces.as_usize_range()],
                )),
            ),
            field(4, OwnedValue::U32(record.virtual_capacity)),
            field(5, OwnedValue::RecordVector(virtual_entries)),
            field(6, OwnedValue::RecordVector(virtual_exits)),
        ])
    });
    let parking_spaces = lir
        .parking_spaces
        .iter()
        .map(|record| parking_space_row(record, &lane_length_mm))
        .collect::<Result<Vec<_>, PortableEmissionError>>()?;
    let lane_groups = lir.lane_groups.iter().map(|record| {
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::U32(record.road_section.raw())),
            field(
                4,
                OwnedValue::OrdinalVectorU32(ordinals(
                    &lir.lane_group_members[record.members.as_usize_range()],
                )),
            ),
        ])
    });
    let facility_bands = lir.facility_bands.iter().map(|record| {
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::U32(record.road_corridor.raw())),
            field(4, OwnedValue::Utf8(record.kind_id.clone())),
        ])
    });
    let participant_classes = lir.participant_classes.iter().map(|record| {
        let mut fields = vec![
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
        ];
        if let Some(parent) = record.parent {
            fields.push(field(3, OwnedValue::U32(parent.raw())));
        }
        fields.extend([
            field(4, OwnedValue::U32(record.depth)),
            field(5, OwnedValue::U32(record.subtree_enter)),
            field(6, OwnedValue::U32(record.subtree_exit)),
        ]);
        row(fields)
    });
    let access_rules = lir.access_rules.iter().map(|record| {
        let (target_kind, target_ordinal) = match record.target {
            crate::lir::LirAccessTarget::LaneEdge(ordinal) => (0, ordinal.raw()),
            crate::lir::LirAccessTarget::LaneGroup(ordinal) => (1, ordinal.raw()),
            crate::lir::LirAccessTarget::RoadSection(ordinal) => (2, ordinal.raw()),
            crate::lir::LirAccessTarget::ManeuverPath(ordinal) => (3, ordinal.raw()),
        };
        let effect = match record.effect {
            laneflow_static_contract::AccessEffect::Deny => 0,
            laneflow_static_contract::AccessEffect::Allow => 1,
            _ => unreachable!("validated LIR only stores the closed v1 access effects"),
        };
        let mut fields = vec![
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::U8(target_kind)),
            field(4, OwnedValue::U32(target_ordinal)),
            field(5, OwnedValue::U8(effect)),
            field(
                6,
                OwnedValue::OrdinalVectorU32(ordinals(
                    &lir.access_rule_participant_classes
                        [record.participant_classes.as_usize_range()],
                )),
            ),
        ];
        if let Some(regulation) = &record.regulation {
            let mut regulation_fields = vec![
                field(1, OwnedValue::Utf8(regulation.jurisdiction.clone())),
                field(2, OwnedValue::Utf8(regulation.version.clone())),
            ];
            if let Some(source) = &regulation.source {
                regulation_fields.push(field(3, OwnedValue::Utf8(source.clone())));
            }
            fields.push(field(
                7,
                OwnedValue::RecordVector(vec![row(regulation_fields)].into_boxed_slice()),
            ));
        }
        fields.push(field(8, OwnedValue::I32(record.priority)));
        row(fields)
    });
    let vehicle_profiles = lir
        .vehicle_profiles
        .iter()
        .map(vehicle_profile_row)
        .collect::<Result<Vec<_>, PortableEmissionError>>()?;
    let conflict_zones = lir.conflict_zones.iter().map(|record| {
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::U32(record.junction.raw())),
        ])
    });
    let canonical_frames = lir.canonical_frames.iter().map(|record| {
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
        ])
    });
    let participant_streams = lir.participant_streams.iter().map(|record| {
        let passages = lir.conflict_passages[record.passages.as_usize_range()]
            .iter()
            .map(conflict_passage_row)
            .collect();
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::U32(record.junction.raw())),
            field(4, OwnedValue::U32(record.maneuver_path.raw())),
            field(5, OwnedValue::RecordVector(passages)),
        ])
    });

    Ok(vec![
        table(1, road_corridors),
        table(2, road_sections),
        table(3, authoring_lanes),
        table(4, lane_edges),
        table(5, junctions),
        table(6, movements),
        table(7, maneuver_paths),
        table(8, maneuver_gates),
        table(9, waiting_zones),
        table(10, stop_lines),
        table(11, signal_groups),
        table(12, signal_controllers),
        table(13, signal_phases),
        table(14, parking_facilities),
        table(15, parking_spaces),
        table(16, lane_groups),
        table(17, facility_bands),
        table(18, participant_classes),
        table(19, access_rules),
        table(20, vehicle_profiles),
        table(21, conflict_zones),
        table(22, canonical_frames),
        table(23, participant_streams),
        // 两个正式前端的策略声明由 W2 接入；无声明的制品仍必须发射完整的空表。
        table(24, []),
    ])
}

fn conflict_passage_row(passage: &crate::lir::LirConflictPassage) -> OwnedRow {
    let (entry_kind, entry_reference) = conflict_anchor_reference(passage.entry.reference);
    let (exit_kind, exit_reference) = conflict_anchor_reference(passage.exit.reference);
    let mut fields = vec![
        field(1, OwnedValue::U32(passage.conflict_zone.raw())),
        field(2, OwnedValue::U8(entry_kind)),
        field(3, OwnedValue::U32(entry_reference)),
    ];
    if let Some(progress_mm) = passage.entry.progress_mm {
        fields.push(field(4, OwnedValue::U32(progress_mm)));
    }
    fields.extend([
        field(5, OwnedValue::U8(exit_kind)),
        field(6, OwnedValue::U32(exit_reference)),
    ]);
    if let Some(progress_mm) = passage.exit.progress_mm {
        fields.push(field(7, OwnedValue::U32(progress_mm)));
    }
    row(fields)
}

fn conflict_anchor_reference(reference: crate::lir::LirPathAnchorReference) -> (u8, u32) {
    match reference {
        crate::lir::LirPathAnchorReference::Gate(gate) => (0, gate.raw()),
        crate::lir::LirPathAnchorReference::EdgeBoundary(boundary_index) => (1, boundary_index),
        crate::lir::LirPathAnchorReference::Interior { path_edge_index } => (2, path_edge_index),
    }
}

/// 交通边长在 HIR 空间冻结时已提交为写出毫米。
fn closed_lane_edge_length_mm(
    _lir: &crate::lir::LirUnit,
    record: &crate::lir::LirLaneEdge,
) -> Result<u32, PortableEmissionError> {
    if record.length_mm < laneflow_static_contract::MIN_LANE_EDGE_LENGTH_MM
        || record.length_mm > laneflow_static_contract::MAX_LANE_EDGE_LENGTH_MM
    {
        return Err(PortableEmissionError::InternalBindingMismatch);
    }
    Ok(record.length_mm)
}

fn parking_anchor_progress_mm(
    progress_mm: u32,
    edge: laneflow_static_contract::LaneEdgeOrdinal,
    lane_length_mm: &[u32],
) -> Result<u32, PortableEmissionError> {
    let length_mm = *lane_length_mm
        .get(edge.index())
        .ok_or(PortableEmissionError::InternalBindingMismatch)?;
    let min_mm = laneflow_static_contract::PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM;
    let max_mm = length_mm.saturating_sub(min_mm);
    if !(min_mm..=max_mm).contains(&progress_mm) {
        return Err(PortableEmissionError::InternalBindingMismatch);
    }
    Ok(progress_mm)
}

fn parking_space_row(
    record: &crate::lir::LirParkingSpace,
    lane_length_mm: &[u32],
) -> Result<OwnedRow, PortableEmissionError> {
    let mut fields = vec![
        field(1, OwnedValue::U32(record.ordinal.raw())),
        field(
            2,
            OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
        ),
    ];
    if let Some(area) = record.parking_facility {
        fields.push(field(3, OwnedValue::U32(area.raw())));
    }
    fields.extend([
        field(4, OwnedValue::U32(record.entry.lane_edge.raw())),
        field(
            5,
            OwnedValue::U32(parking_anchor_progress_mm(
                record.entry.progress_mm,
                record.entry.lane_edge,
                lane_length_mm,
            )?),
        ),
        field(6, OwnedValue::U32(record.exit.lane_edge.raw())),
        field(
            7,
            OwnedValue::U32(parking_anchor_progress_mm(
                record.exit.progress_mm,
                record.exit.lane_edge,
                lane_length_mm,
            )?),
        ),
        field(8, OwnedValue::I32(record.geometry.lateral_offset_mm)),
        field(9, OwnedValue::F32(record.geometry.heading_offset_radians)),
        field(10, OwnedValue::U32(record.geometry.length_mm)),
        field(11, OwnedValue::U32(record.geometry.width_mm)),
    ]);
    Ok(row(fields))
}

fn vehicle_profile_row(
    record: &crate::lir::LirVehicleProfile,
) -> Result<OwnedRow, PortableEmissionError> {
    Ok(row([
        field(1, OwnedValue::U32(record.ordinal.raw())),
        field(
            2,
            OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
        ),
        field(3, OwnedValue::U32(record.participant_class.raw())),
        field(4, OwnedValue::U32(record.length_mm)),
        field(5, OwnedValue::U32(record.desired_speed_mm_s)),
        field(6, OwnedValue::U32(record.min_gap_mm)),
        field(7, OwnedValue::F32(record.time_headway_seconds)),
        field(
            8,
            OwnedValue::F32(record.max_acceleration_meters_per_second_squared),
        ),
        field(
            9,
            OwnedValue::F32(record.comfortable_deceleration_meters_per_second_squared),
        ),
        field(
            10,
            OwnedValue::F32(record.emergency_deceleration_meters_per_second_squared),
        ),
    ]))
}

fn canonical_relation_tables(lir: &crate::lir::LirUnit) -> Vec<OwnedTable> {
    let junction_internal_edges = lir.junction_internal_edges.iter().map(|record| {
        row([
            field(1, OwnedValue::U32(record.edge.raw())),
            field(2, OwnedValue::U32(record.junction.raw())),
        ])
    });
    vec![
        table(1, junction_internal_edges),
        table(2, []),
        table(3, []),
        table(4, []),
        table(5, []),
    ]
}

fn geometry_relation_flags(output: &CompilationOutput) -> BTreeMap<(u32, u8, u32), bool> {
    output
        .source_map_input()
        .spatial_relation_sources()
        .map(|source| {
            (
                (
                    source.owner_ordinal().raw(),
                    source_relation_role_code(source.role()),
                    source.local_index(),
                ),
                source.geometry_source_ranges().len() != 0,
            )
        })
        .collect()
}

fn point_rows(points: &[crate::lir::LirCanonicalPoint3F32]) -> Box<[OwnedRow]> {
    points
        .iter()
        .map(|point| {
            row([
                field(1, OwnedValue::F32(point.x)),
                field(2, OwnedValue::F32(point.y)),
                field(3, OwnedValue::F32(point.z)),
            ])
        })
        .collect()
}

fn lane_edge_geometry_rows(output: &CompilationOutput, direction_profile: u8) -> Vec<OwnedRow> {
    let lir = output.lir().unit();
    let flags = geometry_relation_flags(output);
    let mut next_local_index_by_frame = vec![0_u32; lir.canonical_frames.len()];
    lir.lane_edge_geometries
        .iter()
        .enumerate()
        .map(|(lane_edge, geometry)| {
            let frame = geometry.canonical_frame.raw();
            let local_index = next_local_index_by_frame[geometry.canonical_frame.index()];
            next_local_index_by_frame[geometry.canonical_frame.index()] += 1;
            let applies = flags
                .get(&(
                    frame,
                    source_relation_role_code(
                        crate::source_map::SourceRelationRole::CanonicalFrameLaneEdgeGeometry,
                    ),
                    local_index,
                ))
                .copied()
                .unwrap_or(false);
            debug_assert!(direction_profile != 0 || !applies);
            let segments = lir.spatial_segments[geometry.segments.as_usize_range()]
                .iter()
                .map(|segment| {
                    row([
                        field(1, OwnedValue::F32(segment.length_meters)),
                        field(2, OwnedValue::F32(segment.cumulative_end_meters)),
                        field(3, OwnedValue::F32(segment.tangent[0])),
                        field(4, OwnedValue::F32(segment.tangent[1])),
                        field(5, OwnedValue::F32(segment.tangent[2])),
                        field(6, OwnedValue::F32(segment.up[0])),
                        field(7, OwnedValue::F32(segment.up[1])),
                        field(8, OwnedValue::F32(segment.up[2])),
                    ])
                })
                .collect();
            row([
                field(
                    1,
                    OwnedValue::U32(
                        u32::try_from(lane_edge)
                            .expect("compile limits cap geometry counts at u32"),
                    ),
                ),
                field(2, OwnedValue::U32(frame)),
                field(3, OwnedValue::F32(geometry.arc_length_meters)),
                field(
                    4,
                    OwnedValue::RecordVector(point_rows(
                        &lir.canonical_points[geometry.points.as_usize_range()],
                    )),
                ),
                field(5, OwnedValue::RecordVector(segments)),
                field(6, OwnedValue::U8(u8::from(applies))),
            ])
        })
        .collect()
}

fn facility_band_geometry_rows(output: &CompilationOutput, direction_profile: u8) -> Vec<OwnedRow> {
    let lir = output.lir().unit();
    let flags = geometry_relation_flags(output);
    let mut next_local_index_by_frame = vec![0_u32; lir.canonical_frames.len()];
    lir.facility_band_geometries
        .iter()
        .map(|geometry| {
            let frame = geometry.canonical_frame.raw();
            let local_index = next_local_index_by_frame[geometry.canonical_frame.index()];
            next_local_index_by_frame[geometry.canonical_frame.index()] += 1;
            let applies = flags
                .get(&(
                    frame,
                    source_relation_role_code(
                        crate::source_map::SourceRelationRole::CanonicalFrameFacilityBandGeometry,
                    ),
                    local_index,
                ))
                .copied()
                .unwrap_or(false);
            debug_assert!(direction_profile != 0 || !applies);
            row([
                field(1, OwnedValue::U32(geometry.facility_band.raw())),
                field(2, OwnedValue::U32(frame)),
                field(
                    3,
                    OwnedValue::RecordVector(point_rows(
                        &lir.canonical_points[geometry.points.as_usize_range()],
                    )),
                ),
                field(4, OwnedValue::U8(u8::from(applies))),
            ])
        })
        .collect()
}

fn conflict_zone_region_rows(lir: &crate::lir::LirUnit) -> Vec<OwnedRow> {
    lir.conflict_zone_regions
        .iter()
        .map(|region| {
            let ring = lir.conflict_region_points[region.ring_xz.as_usize_range()]
                .iter()
                .map(|point| {
                    row([
                        field(1, OwnedValue::F32(point.x)),
                        field(2, OwnedValue::F32(point.z)),
                    ])
                })
                .collect();
            row([
                field(1, OwnedValue::U32(region.conflict_zone.raw())),
                field(2, OwnedValue::U32(region.canonical_frame.raw())),
                field(3, OwnedValue::F32(region.min_y)),
                field(4, OwnedValue::F32(region.max_y)),
                field(5, OwnedValue::RecordVector(ring)),
            ])
        })
        .collect()
}
