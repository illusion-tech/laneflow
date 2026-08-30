use super::*;

use laneflow_static_contract::PortableFieldType;

fn candidate_mutations(field_type: PortableFieldType, current: &[u8]) -> Vec<Vec<u8>> {
    let mut values = Vec::new();
    match field_type {
        PortableFieldType::U8 => {
            values.extend((0_u8..=3).map(|value| vec![value]));
        }
        PortableFieldType::U16 => {
            values.extend((0_u16..=36).map(|value| value.to_le_bytes().to_vec()));
        }
        PortableFieldType::U32 => {
            values.extend((0_u32..=32).map(|value| value.to_le_bytes().to_vec()));
            values.extend(
                [
                    100_u32, 101, 200, 1_000, 4_500, 10_000, 12_000, 20_000, 100_000,
                ]
                .map(|value| value.to_le_bytes().to_vec()),
            );
            if let Ok(current) = <[u8; 4]>::try_from(current) {
                let n = u32::from_le_bytes(current);
                for delta in [1_u32, 2, 10, 100] {
                    values.push(n.saturating_add(delta).to_le_bytes().to_vec());
                    values.push(n.saturating_sub(delta).to_le_bytes().to_vec());
                }
            }
        }
        PortableFieldType::U64 => {
            values.extend(
                [1_u64, 2, 10, 100, 1_000, 5_000, 30_000, 35_000, 40_000]
                    .map(|value| value.to_le_bytes().to_vec()),
            );
        }
        PortableFieldType::I32 => {
            values.extend([-8_i32, -7, -1, 0, 1, 7, 8].map(|value| value.to_le_bytes().to_vec()));
        }
        PortableFieldType::F32 => {
            values.extend(
                [0.25_f32, 0.5, 1.0, 2.0, 3.0, 5.0, 10.0]
                    .map(|value| value.to_bits().to_le_bytes().to_vec()),
            );
        }
        PortableFieldType::F64 => {
            values.extend(
                [
                    -3.0_f64, -1.0, -0.25, 0.0, 0.25, 0.5, 1.0, 2.0, 3.0, 5.0, 10.0, 20.0,
                ]
                .map(|value| value.to_bits().to_le_bytes().to_vec()),
            );
        }
        PortableFieldType::Utf8 => {
            for candidate in [
                "motorLane",
                "x-lane-ab",
                "sidewalk",
                "shoulder",
                "facilityStrip",
                "plantingStrip",
            ] {
                if candidate.len() == current.len() {
                    values.push(candidate.as_bytes().to_vec());
                }
            }
            values.push(vec![b'a'; current.len()]);
        }
        PortableFieldType::Bytes
        | PortableFieldType::OrdinalVectorU32
        | PortableFieldType::RecordVector => {
            for index in 4..current.len() {
                for replacement in [
                    current[index] ^ 1,
                    0,
                    1,
                    2,
                    if current[index] == b'a' { b'b' } else { b'a' },
                ] {
                    let mut candidate = current.to_vec();
                    candidate[index] = replacement;
                    values.push(candidate);
                }
            }
        }
        PortableFieldType::StableId128 | PortableFieldType::Sha256 => {
            for index in 0..current.len() {
                let mut candidate = current.to_vec();
                candidate[index] ^= 1;
                values.push(candidate);
            }
        }
    }
    values.retain(|value| value.as_slice() != current);
    values
}

fn only_expected_field_change(
    candidate: &crate::PortablePublicationCandidate,
    expected_section: u32,
    expected_change_kind: u8,
    entity_kind: EntityKind,
    stable_id: [u8; 16],
    field_tag: u16,
) -> bool {
    let diff = registry(
        candidate.semantic_diff().bytes(),
        PortableObjectKind::SemanticDiff,
    );
    let entity_changes = diff.section(1).unwrap().table(0).unwrap();
    let relation_changes = diff.section(2).unwrap().table(0).unwrap();
    let geometry_changes = diff.section(3).unwrap().table(0).unwrap();
    let static_rule_changes = diff.section(4).unwrap().table(0).unwrap();
    let spatial_changes = diff.section(5).unwrap().table(0).unwrap();
    let expected = diff.section(expected_section).unwrap().table(0).unwrap();
    if expected.row_count() != 1
        || relation_changes.row_count() != 0
        || geometry_changes.row_count() != 0
        || spatial_changes.row_count() != 0
        || (expected_section == 1 && static_rule_changes.row_count() != 0)
        || (expected_section == 4 && entity_changes.row_count() != 0)
    {
        return false;
    }
    let row = expected.row(0).unwrap();
    field_u8(row, 1) == expected_change_kind
        && field_u16(row, 2) == entity_kind.code()
        && field_stable_id(row, 4) == stable_id
        && field_u16(row, 6) == field_tag
        && row.field_by_tag(9).is_some()
        && row.field_by_tag(10).is_some()
}

fn prove_field_change(
    output: &CompilationOutput,
    base_bytes: &[u8],
    entity_kind: EntityKind,
    field_tag: u16,
    expected_section: u32,
    expected_change_kind: u8,
) {
    let table_ordinal = entity_table_ordinal(entity_kind);
    let artifact = registry(base_bytes, PortableObjectKind::CanonicalArtifact);
    let table = artifact.section(2).unwrap().table(table_ordinal).unwrap();
    let row_ordinal = (0..table.row_count())
        .find(|ordinal| {
            table
                .row(*ordinal)
                .unwrap()
                .field_by_tag(field_tag)
                .is_some()
        })
        .unwrap_or_else(|| panic!("{entity_kind:?} has no emitted field {field_tag}"));
    let row = table.row(row_ordinal).unwrap();
    let stable_id = field_stable_id(row, 2);
    let field = row.field_by_tag(field_tag).unwrap();
    let field_type = field.field_type();
    let current = field.value_bytes().to_vec();
    let range = field_value_range(
        base_bytes,
        PortableObjectKind::CanonicalArtifact,
        2,
        table_ordinal,
        row_ordinal,
        field_tag,
    );
    let provenance = full_spatial_portable_fixture_provenance();
    for replacement in candidate_mutations(field_type, &current) {
        if replacement.len() != range.len() {
            continue;
        }
        let mut bytes = base_bytes.to_vec();
        bytes[range.clone()].copy_from_slice(&replacement);
        refresh_chunk_digest_containing(
            &mut bytes,
            PortableObjectKind::CanonicalArtifact,
            range.start,
        );
        let Ok(base) = preflight_object_values(
            &bytes,
            PortableObjectKind::CanonicalArtifact,
            FormatLimits::HARD,
        ) else {
            continue;
        };
        let Ok(candidate) = crate::emit_portable_candidate(
            output,
            &provenance,
            FormatLimits::HARD,
            crate::PortableDiffBase::Artifact(base),
        ) else {
            continue;
        };
        if only_expected_field_change(
            &candidate,
            expected_section,
            expected_change_kind,
            entity_kind,
            stable_id,
            field_tag,
        ) {
            return;
        }
    }
    panic!(
        "no valid in-memory mutation proved {entity_kind:?} tag {field_tag} ({field_type:?}) in LFSD section {expected_section}"
    );
}

#[test]
fn diff_every_registered_entity_and_static_rule_field_uses_its_frozen_change_class() {
    let output = full_spatial_portable_fixture_output();
    for (kind, tags) in [
        (EntityKind::RoadSection, &[4_u16][..]),
        (EntityKind::LaneEdge, &[3, 4][..]),
        (EntityKind::ManeuverGate, &[4][..]),
        (EntityKind::StopLine, &[3][..]),
        (EntityKind::ParkingSpace, &[5, 7, 8, 9, 10, 11][..]),
        (EntityKind::FacilityBand, &[4][..]),
        (EntityKind::ParticipantClass, &[4][..]),
        (EntityKind::VehicleProfile, &[4, 5, 6, 7, 8, 9, 10][..]),
    ] {
        for tag in tags {
            prove_field_change(&output, FULL_SPATIAL_EXPECTED_LFCA, kind, *tag, 1, 2);
        }
    }
    for (kind, tags) in [
        (EntityKind::WaitingZone, &[4_u16, 5, 6][..]),
        (EntityKind::SignalController, &[3, 4][..]),
        (EntityKind::SignalPhase, &[4, 5][..]),
        (EntityKind::AccessRule, &[5, 7, 8][..]),
    ] {
        for tag in tags {
            prove_field_change(&output, FULL_SPATIAL_EXPECTED_LFCA, kind, *tag, 4, 0);
        }
    }
}

fn extra_road_section_module() -> SyntheticModule {
    let edge_points = [
        CanonicalPoint3F32Input {
            x: 0.0,
            y: 7.0,
            z: 0.0,
        },
        CanonicalPoint3F32Input {
            x: 7.0,
            y: 7.0,
            z: 0.0,
        },
    ];
    let edge_geometries = [LaneEdgeGeometryInput {
        lane_edge: LaneEdgeReference::local("extra-edge"),
        centerline_points: &edge_points,
    }];
    // Keep the namespace length equal to the fixture namespace while sorting before it, so the
    // insertion deterministically renumbers retained typed ordinals under Identity v1 ordering.
    let mut builder = portable_fixture_builder("city/portable-aaaa-spatial", "extra.document");
    builder
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "extra-edge",
            length_meters: 7.0,
            speed_limit_meters_per_second: 7.0,
            successors: &[],
        })
        .unwrap()
        .add_road_section(RoadSectionInput {
            road_section_key: "extra-section",
            kind_id: "motorLane",
            lanes: &[AuthoringLaneInput {
                authoring_lane_key: "extra-lane",
                edge_chain: &[LaneEdgeReference::local("extra-edge")],
                lane_group: None,
            }],
        })
        .unwrap()
        .add_road_corridor(RoadCorridorInput {
            road_corridor_key: "extra-corridor",
            reference_section: RoadSectionReference::local("extra-section"),
            elements: &[CorridorElementReference::road_section(
                RoadSectionReference::local("extra-section"),
            )],
        })
        .unwrap()
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "extra-frame",
            lane_edge_geometries: &edge_geometries,
        })
        .unwrap();
    builder.finish().unwrap()
}

fn augmented_output() -> CompilationOutput {
    Compiler::new()
        .compile(unit([
            portable_fixture_full_spatial_module(),
            extra_road_section_module(),
        ]))
        .unwrap()
}

#[test]
fn diff_road_corridor_stable_reference_uses_stable_identity_not_raw_ordinal() {
    let output = augmented_output();
    let provenance = full_spatial_portable_fixture_provenance();
    let candidate = crate::emit_portable_candidate(
        &output,
        &provenance,
        FormatLimits::HARD,
        crate::PortableDiffBase::Genesis,
    )
    .unwrap();
    prove_field_change(
        &output,
        candidate.canonical_artifact().bytes(),
        EntityKind::RoadCorridor,
        3,
        1,
        2,
    );
}

#[test]
fn diff_signal_control_kind_and_scalar_relation_change_remain_separate() {
    let output = full_spatial_portable_fixture_output();
    let provenance = full_spatial_portable_fixture_provenance();
    let artifact = registry(
        FULL_SPATIAL_EXPECTED_LFCA,
        PortableObjectKind::CanonicalArtifact,
    );
    let gates = artifact
        .section(2)
        .unwrap()
        .table(entity_table_ordinal(EntityKind::ManeuverGate))
        .unwrap();
    let gate_row = (0..gates.row_count())
        .find(|ordinal| gates.row(*ordinal).unwrap().field_by_tag(7).is_some())
        .unwrap();
    let gate_stable_id = field_stable_id(gates.row(gate_row).unwrap(), 2);

    let mut base_bytes = FULL_SPATIAL_EXPECTED_LFCA.to_vec();
    let control_kind = field_value_range(
        &base_bytes,
        PortableObjectKind::CanonicalArtifact,
        2,
        entity_table_ordinal(EntityKind::ManeuverGate),
        gate_row,
        6,
    );
    remove_field(
        &mut base_bytes,
        PortableObjectKind::CanonicalArtifact,
        2,
        entity_table_ordinal(EntityKind::ManeuverGate),
        gate_row,
        7,
    );
    base_bytes[control_kind.start] = 0;
    refresh_chunk_digest_containing(
        &mut base_bytes,
        PortableObjectKind::CanonicalArtifact,
        control_kind.start,
    );
    let base = value_checked(&base_bytes, PortableObjectKind::CanonicalArtifact);
    let candidate = crate::emit_portable_candidate(
        &output,
        &provenance,
        FormatLimits::HARD,
        crate::PortableDiffBase::Artifact(base),
    )
    .unwrap();
    let diff = registry(
        candidate.semantic_diff().bytes(),
        PortableObjectKind::SemanticDiff,
    );
    assert_eq!(diff.section(1).unwrap().table(0).unwrap().row_count(), 0);
    let relation_changes = diff.section(2).unwrap().table(0).unwrap();
    assert_eq!(relation_changes.row_count(), 1);
    let relation = relation_changes.row(0).unwrap();
    assert_eq!(field_u8(relation, 1), 0);
    assert_eq!(field_u8(relation, 5), 20);
    assert_eq!(field_stable_id(relation, 3), gate_stable_id);
    let static_changes = diff.section(4).unwrap().table(0).unwrap();
    assert_eq!(static_changes.row_count(), 1);
    let static_change = static_changes.row(0).unwrap();
    assert_eq!(field_u8(static_change, 1), 0);
    assert_eq!(field_u16(static_change, 2), EntityKind::ManeuverGate.code());
    assert_eq!(field_stable_id(static_change, 4), gate_stable_id);
    assert_eq!(field_u16(static_change, 6), 6);
}

#[test]
fn diff_ordinal_only_insertion_does_not_create_retained_field_or_geometry_modify() {
    let base_output = Compiler::new()
        .compile(unit([portable_fixture_full_spatial_module()]))
        .unwrap();
    let target_output = augmented_output();
    let provenance = full_spatial_portable_fixture_provenance();
    let base_candidate = crate::emit_portable_candidate(
        &base_output,
        &provenance,
        FormatLimits::HARD,
        crate::PortableDiffBase::Genesis,
    )
    .unwrap();
    let base = value_checked(
        base_candidate.canonical_artifact().bytes(),
        PortableObjectKind::CanonicalArtifact,
    );
    let candidate = crate::emit_portable_candidate(
        &target_output,
        &provenance,
        FormatLimits::HARD,
        crate::PortableDiffBase::Artifact(base),
    )
    .unwrap();

    let base_artifact = registry(
        base_candidate.canonical_artifact().bytes(),
        PortableObjectKind::CanonicalArtifact,
    );
    let target_artifact = registry(
        candidate.canonical_artifact().bytes(),
        PortableObjectKind::CanonicalArtifact,
    );
    let identity_ordinals = |view: RegistryCheckedObjectView<'_>| {
        let identities = view.section(1).unwrap().table(0).unwrap();
        (0..identities.row_count())
            .map(|ordinal| {
                let row = identities.row(ordinal).unwrap();
                (
                    (field_u16(row, 1), field_stable_id(row, 3)),
                    field_u32(row, 2),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let base_ordinals = identity_ordinals(base_artifact);
    let target_ordinals = identity_ordinals(target_artifact);
    assert!(base_ordinals.iter().any(|(key, ordinal)| {
        target_ordinals
            .get(key)
            .is_some_and(|target| target != ordinal)
    }));
    let geometry_reference_shifted = [
        (1_u32, EntityKind::LaneEdge),
        (2_u32, EntityKind::FacilityBand),
    ]
    .into_iter()
    .any(|(table_ordinal, subject_kind)| {
        let table = base_artifact
            .section(4)
            .unwrap()
            .table(table_ordinal)
            .unwrap();
        (0..table.row_count()).any(|row_ordinal| {
            let geometry = table.row(row_ordinal).unwrap();
            [
                (subject_kind, field_u32(geometry, 1)),
                (EntityKind::CanonicalFrame, field_u32(geometry, 2)),
            ]
            .into_iter()
            .any(|(kind, raw_ordinal)| {
                base_ordinals
                    .iter()
                    .find(|((candidate_kind, _), ordinal)| {
                        *candidate_kind == kind.code() && **ordinal == raw_ordinal
                    })
                    .and_then(|(key, _)| target_ordinals.get(key))
                    .is_some_and(|target_ordinal| *target_ordinal != raw_ordinal)
            })
        })
    });
    assert!(geometry_reference_shifted);

    let diff = registry(
        candidate.semantic_diff().bytes(),
        PortableObjectKind::SemanticDiff,
    );
    let entity_changes = diff.section(1).unwrap().table(0).unwrap();
    assert!(
        (0..entity_changes.row_count())
            .all(|ordinal| field_u8(entity_changes.row(ordinal).unwrap(), 1) == 0)
    );
    let geometry_changes = diff.section(3).unwrap().table(0).unwrap();
    assert!(
        (0..geometry_changes.row_count())
            .all(|ordinal| field_u8(geometry_changes.row(ordinal).unwrap(), 1) != 2)
    );
}

#[test]
fn diff_gate_transition_field_modify_and_role_move_are_both_reported() {
    let output = full_spatial_portable_fixture_output();
    let provenance = full_spatial_portable_fixture_provenance();
    let artifact = registry(
        FULL_SPATIAL_EXPECTED_LFCA,
        PortableObjectKind::CanonicalArtifact,
    );
    let path_table = artifact
        .section(2)
        .unwrap()
        .table(entity_table_ordinal(EntityKind::ManeuverPath))
        .unwrap();
    let path_row = (0..path_table.row_count())
        .find(|ordinal| {
            path_table
                .row(*ordinal)
                .unwrap()
                .field_by_tag(5)
                .unwrap()
                .value_bytes()
                .len()
                == 12
        })
        .unwrap();
    let gate_table = artifact
        .section(2)
        .unwrap()
        .table(entity_table_ordinal(EntityKind::ManeuverGate))
        .unwrap();
    let gate_ordinals = field_ordinals(path_table.row(path_row).unwrap(), 5);
    assert_eq!(gate_ordinals.len(), 2);
    let first_gate = gate_ordinals[0];
    let second_gate = gate_ordinals[1];
    let original_first_transition = field_u32(gate_table.row(first_gate).unwrap(), 4);
    let original_second_transition = field_u32(gate_table.row(second_gate).unwrap(), 4);
    assert_ne!(original_first_transition, original_second_transition);

    let mut base_bytes = FULL_SPATIAL_EXPECTED_LFCA.to_vec();
    let gate_vector = field_value_range(
        &base_bytes,
        PortableObjectKind::CanonicalArtifact,
        2,
        entity_table_ordinal(EntityKind::ManeuverPath),
        path_row,
        5,
    );
    let first = base_bytes[gate_vector.start + 4..gate_vector.start + 8].to_vec();
    let second = base_bytes[gate_vector.start + 8..gate_vector.start + 12].to_vec();
    base_bytes[gate_vector.start + 4..gate_vector.start + 8].copy_from_slice(&second);
    base_bytes[gate_vector.start + 8..gate_vector.start + 12].copy_from_slice(&first);
    refresh_chunk_digest_containing(
        &mut base_bytes,
        PortableObjectKind::CanonicalArtifact,
        gate_vector.start,
    );
    let first_transition = field_value_range(
        &base_bytes,
        PortableObjectKind::CanonicalArtifact,
        2,
        entity_table_ordinal(EntityKind::ManeuverGate),
        first_gate,
        4,
    );
    let second_transition = field_value_range(
        &base_bytes,
        PortableObjectKind::CanonicalArtifact,
        2,
        entity_table_ordinal(EntityKind::ManeuverGate),
        second_gate,
        4,
    );
    let first_transition_start = first_transition.start;
    let second_transition_start = second_transition.start;
    base_bytes[first_transition].copy_from_slice(&original_second_transition.to_le_bytes());
    base_bytes[second_transition].copy_from_slice(&original_first_transition.to_le_bytes());
    refresh_chunk_digest_containing(
        &mut base_bytes,
        PortableObjectKind::CanonicalArtifact,
        first_transition_start,
    );
    refresh_chunk_digest_containing(
        &mut base_bytes,
        PortableObjectKind::CanonicalArtifact,
        second_transition_start,
    );
    let base = value_checked(&base_bytes, PortableObjectKind::CanonicalArtifact);
    let candidate = crate::emit_portable_candidate(
        &output,
        &provenance,
        FormatLimits::HARD,
        crate::PortableDiffBase::Artifact(base),
    )
    .unwrap();
    let diff = registry(
        candidate.semantic_diff().bytes(),
        PortableObjectKind::SemanticDiff,
    );
    let entity_changes = diff.section(1).unwrap().table(0).unwrap();
    assert_eq!(entity_changes.row_count(), 2);
    assert!((0..entity_changes.row_count()).all(|ordinal| {
        let row = entity_changes.row(ordinal).unwrap();
        field_u8(row, 1) == 2
            && field_u16(row, 2) == EntityKind::ManeuverGate.code()
            && field_u16(row, 6) == 4
    }));
    let relation_changes = diff.section(2).unwrap().table(0).unwrap();
    let role_ten = (0..relation_changes.row_count())
        .filter(|ordinal| field_u8(relation_changes.row(*ordinal).unwrap(), 5) == 10)
        .collect::<Vec<_>>();
    assert_eq!(role_ten.len(), 2);
    assert!(role_ten.into_iter().all(|ordinal| {
        let row = relation_changes.row(ordinal).unwrap();
        field_u8(row, 1) == 2 && field_u32(row, 7) != field_u32(row, 8)
    }));
}
