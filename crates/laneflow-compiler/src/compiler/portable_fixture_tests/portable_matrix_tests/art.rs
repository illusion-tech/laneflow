use super::*;

use sha2::{Digest, Sha256};

use laneflow_format::FormatErrorClass;

#[test]
fn art_candidate_closes_identity_tables_and_all_computed_bindings() {
    let candidate = full_spatial_portable_fixture_candidate();
    let artifact = registry(
        candidate.canonical_artifact().bytes(),
        PortableObjectKind::CanonicalArtifact,
    );
    let source_map = registry(
        candidate.source_map().bytes(),
        PortableObjectKind::SourceMap,
    );
    let diff = registry(
        candidate.semantic_diff().bytes(),
        PortableObjectKind::SemanticDiff,
    );

    let identities = artifact.section(1).unwrap().table(0).unwrap();
    let entities = artifact.section(2).unwrap();
    let constructible = EntityKind::ALL
        .into_iter()
        .filter(|kind| kind.is_constructible())
        .collect::<Vec<_>>();
    assert_eq!(
        entities.table_count(),
        u32::try_from(constructible.len()).unwrap()
    );
    assert_eq!(
        identities.row_count(),
        (0..entities.table_count())
            .map(|ordinal| entities.table(ordinal).unwrap().row_count())
            .sum()
    );
    let identity_keys = (0..identities.row_count())
        .map(|ordinal| {
            let row = identities.row(ordinal).unwrap();
            (
                field_u16(row, 1),
                field_u32(row, 2),
                field_stable_id(row, 3),
            )
        })
        .collect::<std::collections::BTreeSet<_>>();
    let entity_keys = constructible
        .into_iter()
        .enumerate()
        .flat_map(|(table_ordinal, kind)| {
            let table = entities
                .table(u32::try_from(table_ordinal).unwrap())
                .unwrap();
            (0..table.row_count()).map(move |row_ordinal| {
                let row = table.row(row_ordinal).unwrap();
                (kind.code(), field_u32(row, 1), field_stable_id(row, 2))
            })
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(identity_keys, entity_keys);

    let claim = artifact
        .section(7)
        .unwrap()
        .table(0)
        .unwrap()
        .row(0)
        .unwrap();
    assert_eq!(
        field_sha256(claim, 1),
        candidate.network_revision().into_digest().into_bytes()
    );
    let source_binding = source_map
        .section(0)
        .unwrap()
        .table(0)
        .unwrap()
        .row(0)
        .unwrap();
    assert_eq!(field_u16(source_binding, 1), 1);
    assert_eq!(
        field_sha256(source_binding, 2),
        candidate.network_revision().into_digest().into_bytes()
    );
    assert_eq!(field_u16(source_binding, 3), 3);
    assert_eq!(
        field_sha256(source_binding, 4),
        candidate.canonical_artifact().digest().into_bytes()
    );
    assert_eq!(
        field_u64(source_binding, 5),
        candidate.canonical_artifact().byte_length().get()
    );

    let diff_binding = diff.section(0).unwrap().table(0).unwrap().row(0).unwrap();
    assert_eq!(field_u8(diff_binding, 1), 0);
    assert_eq!(field_u16(diff_binding, 6), 1);
    assert_eq!(
        field_sha256(diff_binding, 7),
        candidate.network_revision().into_digest().into_bytes()
    );
    assert_eq!(
        field_sha256(diff_binding, 8),
        candidate.canonical_artifact().digest().into_bytes()
    );
    assert_eq!(
        field_u64(diff_binding, 9),
        candidate.canonical_artifact().byte_length().get()
    );

    for object in [
        candidate.canonical_artifact(),
        candidate.source_map(),
        candidate.semantic_diff(),
    ] {
        assert_eq!(
            object.digest(),
            sha256_digest(Sha256::digest(object.bytes()).into())
        );
        assert_eq!(
            object.byte_length(),
            exact_byte_length(u64::try_from(object.bytes().len()).unwrap())
        );
        let mut expected_key = String::from("sha256/");
        for byte in object.digest().into_bytes() {
            use std::fmt::Write as _;
            write!(&mut expected_key, "{byte:02x}").unwrap();
        }
        assert_eq!(object.object_key(), expected_key);
    }
}

#[test]
fn art_direct_version_options_profile_and_build_mutations_fail_closed() {
    for tag in 1..=6 {
        let mut bytes = FULL_SPATIAL_EXPECTED_LFCA.to_vec();
        let range = field_value_range(&bytes, PortableObjectKind::CanonicalArtifact, 0, 0, 0, tag);
        let replacement = 4_u16;
        bytes[range].copy_from_slice(&replacement.to_le_bytes());
        assert_eq!(
            preflight_object_values(
                &bytes,
                PortableObjectKind::CanonicalArtifact,
                FormatLimits::HARD,
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::NonCanonicalValue
        );
    }
    for tag in 1..=2 {
        let mut bytes = FULL_SPATIAL_EXPECTED_LFCA.to_vec();
        let range = field_value_range(&bytes, PortableObjectKind::CanonicalArtifact, 5, 0, 0, tag);
        bytes[range].copy_from_slice(&4_u16.to_le_bytes());
        assert_eq!(
            preflight_object_values(
                &bytes,
                PortableObjectKind::CanonicalArtifact,
                FormatLimits::HARD,
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::NonCanonicalValue
        );
    }

    let mut wrong_options = FULL_SPATIAL_EXPECTED_LFCA.to_vec();
    let options = field_value_range(
        &wrong_options,
        PortableObjectKind::CanonicalArtifact,
        6,
        0,
        0,
        4,
    );
    wrong_options[options.start] ^= 1;
    assert_eq!(
        preflight_object_values(
            &wrong_options,
            PortableObjectKind::CanonicalArtifact,
            FormatLimits::HARD,
        )
        .unwrap_err()
        .class(),
        FormatErrorClass::BindingMismatch
    );

    let mut wrong_build = FULL_SPATIAL_EXPECTED_LFCA.to_vec();
    let build = field_value_range(
        &wrong_build,
        PortableObjectKind::CanonicalArtifact,
        6,
        0,
        0,
        1,
    );
    wrong_build[build.start + 1] = b'/';
    assert_eq!(
        preflight_object_values(
            &wrong_build,
            PortableObjectKind::CanonicalArtifact,
            FormatLimits::HARD,
        )
        .unwrap_err()
        .class(),
        FormatErrorClass::NonCanonicalValue
    );

    for (section, tag) in [(4, 2), (6, 6)] {
        let mut mismatched_profile = FULL_SPATIAL_EXPECTED_LFCA.to_vec();
        let range = field_value_range(
            &mismatched_profile,
            PortableObjectKind::CanonicalArtifact,
            section,
            0,
            0,
            tag,
        );
        mismatched_profile[range.start] = 1;
        assert_eq!(
            preflight_object_values(
                &mismatched_profile,
                PortableObjectKind::CanonicalArtifact,
                FormatLimits::HARD,
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::BindingMismatch
        );
    }
}

#[test]
fn art_production_provenance_rejects_noncanonical_environment_data() {
    for invalid in [
        "",
        "build/target",
        "build\\target",
        "build:2026-08-17T00:00:00Z",
        "-build",
    ] {
        assert_eq!(
            crate::PortableEmissionProvenance::try_new(invalid),
            Err(crate::PortableEmissionError::InvalidCompilerBuildId)
        );
    }
    assert_eq!(
        crate::PortableEmissionProvenance::try_new("a".repeat(129)),
        Err(crate::PortableEmissionError::InvalidCompilerBuildId)
    );
    assert!(crate::PortableEmissionProvenance::try_new("build.v1+ci@main-17").is_ok());
}

#[test]
fn art_diff_base_rejects_duplicate_identity_ordinals_entity_mismatch_and_unknown_refs() {
    let output = full_spatial_portable_fixture_output();
    let provenance = full_spatial_portable_fixture_provenance();
    let emit_with_base = |bytes: &[u8]| {
        let base = value_checked(bytes, PortableObjectKind::CanonicalArtifact);
        crate::emit_portable_candidate(
            &output,
            &provenance,
            FormatLimits::HARD,
            crate::PortableDiffBase::Artifact(base),
        )
    };

    let artifact = registry(
        FULL_SPATIAL_EXPECTED_LFCA,
        PortableObjectKind::CanonicalArtifact,
    );
    let identities = artifact.section(1).unwrap().table(0).unwrap();
    let lane_identity_rows = (0..identities.row_count())
        .filter(|ordinal| {
            field_u16(identities.row(*ordinal).unwrap(), 1) == EntityKind::LaneEdge.code()
        })
        .collect::<Vec<_>>();
    assert!(lane_identity_rows.len() >= 2);

    let mut duplicate_stable_id = FULL_SPATIAL_EXPECTED_LFCA.to_vec();
    copy_field_value(
        &mut duplicate_stable_id,
        PortableObjectKind::CanonicalArtifact,
        (1, 0, lane_identity_rows[0], 3),
        (1, 0, lane_identity_rows[1], 3),
    );
    assert_eq!(
        emit_with_base(&duplicate_stable_id),
        Err(crate::PortableEmissionError::DiffBaseSemanticMismatch)
    );

    let mut duplicate_ordinal = FULL_SPATIAL_EXPECTED_LFCA.to_vec();
    copy_field_value(
        &mut duplicate_ordinal,
        PortableObjectKind::CanonicalArtifact,
        (1, 0, lane_identity_rows[0], 2),
        (1, 0, lane_identity_rows[1], 2),
    );
    assert_eq!(
        emit_with_base(&duplicate_ordinal),
        Err(crate::PortableEmissionError::DiffBaseSemanticMismatch)
    );

    let mut mismatched_entity = FULL_SPATIAL_EXPECTED_LFCA.to_vec();
    let entity_stable_id = field_value_range(
        &mismatched_entity,
        PortableObjectKind::CanonicalArtifact,
        2,
        entity_table_ordinal(EntityKind::LaneEdge),
        0,
        2,
    );
    mismatched_entity[entity_stable_id].copy_from_slice(&[0xa5; 16]);
    assert_eq!(
        emit_with_base(&mismatched_entity),
        Err(crate::PortableEmissionError::DiffBaseSemanticMismatch)
    );

    let mut unknown_reference = FULL_SPATIAL_EXPECTED_LFCA.to_vec();
    let reference_section = field_value_range(
        &unknown_reference,
        PortableObjectKind::CanonicalArtifact,
        2,
        entity_table_ordinal(EntityKind::RoadCorridor),
        0,
        3,
    );
    unknown_reference[reference_section].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        emit_with_base(&unknown_reference),
        Err(crate::PortableEmissionError::DiffBaseSemanticMismatch)
    );
}
