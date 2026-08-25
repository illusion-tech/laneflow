use super::*;

const FIXTURE_PROVENANCE: &str = "laneflow-fixture-298-change-set-v1";
const EXPECTED_LFSD: &[u8] =
    include_bytes!("../../../tests/fixtures/portable/lfsd-change-set/expected.lfsd");

fn module(target: bool) -> SyntheticModule {
    let retained_points = [
        CanonicalPoint3F32Input {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        CanonicalPoint3F32Input {
            x: 10.0,
            y: 0.0,
            z: 0.0,
        },
    ];
    let identity_points = [
        CanonicalPoint3F32Input {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
        CanonicalPoint3F32Input {
            x: 5.0,
            y: 1.0,
            z: 0.0,
        },
    ];
    let geometries = [
        LaneEdgeGeometryInput {
            lane_edge: LaneEdgeReference::local("retained-edge"),
            centerline_points: &retained_points,
        },
        LaneEdgeGeometryInput {
            lane_edge: LaneEdgeReference::local("identity-new"),
            centerline_points: &identity_points,
        },
    ];
    let participant_classes = [ParticipantClassReference::local("child-class")];
    let mut builder =
        portable_fixture_builder("city/portable-change-set", "portable-change-set.document");
    builder
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "retained-edge",
            length_meters: 10.0,
            speed_limit_meters_per_second: if target { 12.0 } else { 10.0 },
            successors: &[],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: if target {
                "identity-new"
            } else {
                "identity-old"
            },
            length_meters: 5.0,
            speed_limit_meters_per_second: 5.0,
            successors: &[],
        })
        .unwrap()
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "parent-a",
            extends: None,
        })
        .unwrap()
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "parent-b",
            extends: None,
        })
        .unwrap()
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "child-class",
            extends: Some(ParticipantClassReference::local(if target {
                "parent-b"
            } else {
                "parent-a"
            })),
        })
        .unwrap()
        .add_access_rule(AccessRuleInput {
            access_rule_key: "controlled-access",
            target: AccessRuleTargetInput::LaneEdge(LaneEdgeReference::local("retained-edge")),
            effect: if target {
                AccessEffect::Deny
            } else {
                AccessEffect::Allow
            },
            participant_classes: &participant_classes,
            regulation: None,
            priority: 10,
        })
        .unwrap();
    if target {
        builder
            .add_canonical_frame(CanonicalFrameInput {
                canonical_frame_key: "frame-main",
                lane_edge_geometries: &geometries,
            })
            .unwrap();
    }
    builder.finish().unwrap()
}

fn output(target: bool) -> CompilationOutput {
    Compiler::new().compile(unit([module(target)])).unwrap()
}

fn candidate() -> (
    crate::PortablePublicationCandidate,
    crate::PortablePublicationCandidate,
) {
    let provenance = crate::PortableEmissionProvenance::try_new(FIXTURE_PROVENANCE).unwrap();
    let base_output = output(false);
    let base_candidate = crate::emit_portable_candidate(
        &base_output,
        &provenance,
        laneflow_format::FormatLimits::HARD,
        crate::PortableDiffBase::Genesis,
    )
    .unwrap();
    let base = laneflow_format::preflight_object_values(
        base_candidate.canonical_artifact().bytes(),
        laneflow_static_contract::PortableObjectKind::CanonicalArtifact,
        laneflow_format::FormatLimits::HARD,
    )
    .unwrap();
    let target_output = output(true);
    let target_candidate = crate::emit_portable_candidate(
        &target_output,
        &provenance,
        laneflow_format::FormatLimits::HARD,
        crate::PortableDiffBase::Artifact(base),
    )
    .unwrap();
    (base_candidate, target_candidate)
}

fn row_tags(row: laneflow_format::RegistryCheckedRowView<'_>) -> Vec<u16> {
    (0..row.field_count())
        .map(|ordinal| row.field(ordinal).unwrap().tag())
        .collect()
}

#[test]
fn portable_change_set_diff_matches_frozen_exact_bytes() {
    let (base, target) = candidate();
    assert_eq!(target.semantic_diff().bytes(), EXPECTED_LFSD);
    assert_eq!(
        target.semantic_diff().byte_length(),
        exact_byte_length(2_455)
    );
    assert_eq!(
        target.semantic_diff().object_key(),
        "sha256/81bf319ee57d9b85b6607c3804a0a46e3810c3e6009d37d80820b88d583cfd47"
    );
    assert_eq!(
        base.canonical_artifact().object_key(),
        "sha256/0e05b0a761d47c6c6b9a5b6b84cba7f949fb2e4358011e3b46e7f702505a67fd"
    );
    assert_eq!(
        base.canonical_artifact().byte_length(),
        exact_byte_length(3_204)
    );
    assert_eq!(
        target.canonical_artifact().object_key(),
        "sha256/16abfbd35d81b7b03814fa3707d56269ff4be0e2b3da61bcdcbfd6e036d22f09"
    );
    assert_eq!(
        target.canonical_artifact().byte_length(),
        exact_byte_length(4_234)
    );

    let diff = laneflow_format::preflight_object_values(
        EXPECTED_LFSD,
        laneflow_static_contract::PortableObjectKind::SemanticDiff,
        laneflow_format::FormatLimits::HARD,
    )
    .unwrap()
    .registry_view();

    let binding = diff.section(0).unwrap().table(0).unwrap().row(0).unwrap();
    assert!(matches!(
        binding.field_by_tag(1).unwrap().value().unwrap(),
        laneflow_format::RegistryCheckedFieldValue::U8(1)
    ));
    assert_eq!(
        binding.field_by_tag(3).unwrap().value_bytes(),
        base.network_revision().as_digest().as_bytes()
    );
    assert_eq!(
        binding.field_by_tag(4).unwrap().value_bytes(),
        base.canonical_artifact().digest().as_bytes()
    );
    assert!(matches!(
        binding.field_by_tag(5).unwrap().value().unwrap(),
        laneflow_format::RegistryCheckedFieldValue::U64(3_204)
    ));
    assert_eq!(
        binding.field_by_tag(7).unwrap().value_bytes(),
        target.network_revision().as_digest().as_bytes()
    );
    assert_eq!(
        binding.field_by_tag(8).unwrap().value_bytes(),
        target.canonical_artifact().digest().as_bytes()
    );
    assert!(matches!(
        binding.field_by_tag(9).unwrap().value().unwrap(),
        laneflow_format::RegistryCheckedFieldValue::U64(4_234)
    ));

    let entity_changes = diff.section(1).unwrap().table(0).unwrap();
    assert_eq!(entity_changes.row_count(), 4);
    let identity_add = entity_changes.row(0).unwrap();
    let frame_add = entity_changes.row(1).unwrap();
    let identity_remove = entity_changes.row(2).unwrap();
    let retained_modify = entity_changes.row(3).unwrap();
    assert_eq!(row_tags(identity_add), [1, 2, 4, 10]);
    assert_eq!(row_tags(frame_add), [1, 2, 4, 10]);
    assert_eq!(row_tags(identity_remove), [1, 2, 4, 9]);
    assert_eq!(row_tags(retained_modify), [1, 2, 4, 6, 9, 10]);
    assert!(matches!(
        identity_add.field_by_tag(1).unwrap().value().unwrap(),
        laneflow_format::RegistryCheckedFieldValue::U8(0)
    ));
    assert!(matches!(
        identity_remove.field_by_tag(1).unwrap().value().unwrap(),
        laneflow_format::RegistryCheckedFieldValue::U8(1)
    ));
    assert_eq!(
        identity_add.field_by_tag(4).unwrap().value_bytes(),
        [
            0x96, 0x3f, 0xfe, 0x80, 0xb6, 0x94, 0x38, 0xa4, 0xbc, 0xee, 0x1b, 0x50, 0xe4, 0x55,
            0xe6, 0x10,
        ]
    );
    assert_eq!(
        identity_remove.field_by_tag(4).unwrap().value_bytes(),
        [
            0xa5, 0xf9, 0x0a, 0x50, 0x6f, 0x44, 0x46, 0xb0, 0x60, 0x12, 0xf6, 0xae, 0x8e, 0xb2,
            0xdc, 0x8f,
        ]
    );
    assert_ne!(
        identity_add.field_by_tag(4).unwrap().value_bytes(),
        identity_remove.field_by_tag(4).unwrap().value_bytes()
    );
    assert!(matches!(
        retained_modify.field_by_tag(1).unwrap().value().unwrap(),
        laneflow_format::RegistryCheckedFieldValue::U8(2)
    ));
    assert!(matches!(
        retained_modify.field_by_tag(6).unwrap().value().unwrap(),
        laneflow_format::RegistryCheckedFieldValue::U16(4)
    ));
    assert_eq!(
        retained_modify.field_by_tag(9).unwrap().value_bytes(),
        10_000_u32.to_le_bytes()
    );
    assert_eq!(
        retained_modify.field_by_tag(10).unwrap().value_bytes(),
        12_000_u32.to_le_bytes()
    );

    let relation_changes = diff.section(2).unwrap().table(0).unwrap();
    assert_eq!(relation_changes.row_count(), 1);
    let reconnect = relation_changes.row(0).unwrap();
    assert_eq!(row_tags(reconnect), [1, 2, 3, 5, 7, 8, 9, 10]);
    assert!(matches!(
        reconnect.field_by_tag(1).unwrap().value().unwrap(),
        laneflow_format::RegistryCheckedFieldValue::U8(3)
    ));
    assert!(matches!(
        reconnect.field_by_tag(5).unwrap().value().unwrap(),
        laneflow_format::RegistryCheckedFieldValue::U8(24)
    ));
    assert_ne!(
        reconnect.field_by_tag(9).unwrap().value_bytes(),
        reconnect.field_by_tag(10).unwrap().value_bytes()
    );

    let geometry_changes = diff.section(3).unwrap().table(0).unwrap();
    assert_eq!(geometry_changes.row_count(), 2);
    for ordinal in 0..geometry_changes.row_count() {
        let add = geometry_changes.row(ordinal).unwrap();
        assert_eq!(row_tags(add), [1, 2, 4, 10]);
        assert!(matches!(
            add.field_by_tag(1).unwrap().value().unwrap(),
            laneflow_format::RegistryCheckedFieldValue::U8(0)
        ));
    }

    let rule_changes = diff.section(4).unwrap().table(0).unwrap();
    assert_eq!(rule_changes.row_count(), 1);
    let rule_modify = rule_changes.row(0).unwrap();
    assert_eq!(row_tags(rule_modify), [1, 2, 4, 6, 9, 10]);
    assert!(matches!(
        rule_modify.field_by_tag(6).unwrap().value().unwrap(),
        laneflow_format::RegistryCheckedFieldValue::U16(5)
    ));
    assert_eq!(rule_modify.field_by_tag(9).unwrap().value_bytes(), [1]);
    assert_eq!(rule_modify.field_by_tag(10).unwrap().value_bytes(), [0]);

    let spatial_changes = diff.section(5).unwrap().table(0).unwrap();
    assert_eq!(spatial_changes.row_count(), 1);
    let spatial_modify = spatial_changes.row(0).unwrap();
    assert_eq!(row_tags(spatial_modify), [1, 2, 3]);
    assert!(matches!(
        spatial_modify.field_by_tag(1).unwrap().value().unwrap(),
        laneflow_format::RegistryCheckedFieldValue::U8(1)
    ));
    assert_ne!(
        spatial_modify.field_by_tag(2).unwrap().value_bytes(),
        spatial_modify.field_by_tag(3).unwrap().value_bytes()
    );
}

#[test]
fn dump_portable_change_set_when_requested() {
    if std::env::var_os("DUMP_PORTABLE").is_none() {
        return;
    }
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/portable/lfsd-change-set");
    std::fs::create_dir_all(&dir).unwrap();
    let (base, target) = candidate();
    std::fs::write(dir.join("expected.lfsd"), target.semantic_diff().bytes()).unwrap();
    std::fs::write(
        dir.join("bindings.txt"),
        format!(
            "lfsd_len={}\nlfsd_key={}\nbase_len={}\nbase_key={}\ntarget_len={}\ntarget_key={}\n",
            target.semantic_diff().bytes().len(),
            target.semantic_diff().object_key(),
            base.canonical_artifact().bytes().len(),
            base.canonical_artifact().object_key(),
            target.canonical_artifact().bytes().len(),
            target.canonical_artifact().object_key(),
        ),
    )
    .unwrap();
}
