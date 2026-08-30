use super::*;

use std::collections::{BTreeMap, BTreeSet};

use laneflow_format::{FormatErrorClass, RegistryCheckedFieldValue};

#[test]
fn map_candidate_closes_modules_documents_locations_and_source_bijections() {
    let output = full_spatial_portable_fixture_output();
    let provenance = full_spatial_portable_fixture_provenance();
    let candidate = crate::emit_portable_candidate(
        &output,
        &provenance,
        FormatLimits::HARD,
        crate::PortableDiffBase::Genesis,
    )
    .unwrap();
    let artifact = registry(
        candidate.canonical_artifact().bytes(),
        PortableObjectKind::CanonicalArtifact,
    );
    let source_map = registry(
        candidate.source_map().bytes(),
        PortableObjectKind::SourceMap,
    );
    let source_section = source_map.section(1).unwrap();
    let module_table = source_section.table(0).unwrap();
    let document_table = source_section.table(1).unwrap();
    let location_table = source_section.table(2).unwrap();

    let module_views = output
        .source_map_input()
        .source_module_sources()
        .collect::<Vec<_>>();
    assert_eq!(
        module_table.row_count(),
        u32::try_from(module_views.len()).unwrap()
    );
    let module_ordinals = module_views
        .iter()
        .enumerate()
        .map(|(ordinal, source)| {
            (
                source.descriptor().authoring_namespace_id(),
                u32::try_from(ordinal).unwrap(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    for (ordinal, source) in module_views.iter().enumerate() {
        let descriptor = source.descriptor();
        let row = module_table.row(u32::try_from(ordinal).unwrap()).unwrap();
        assert_eq!(field_u32(row, 1), u32::try_from(ordinal).unwrap());
        assert_eq!(field_utf8(row, 2), descriptor.authoring_namespace_id());
        assert_eq!(field_u16(row, 3), descriptor.source_language() as u16);
        assert_eq!(
            field_sha256(row, 4),
            *descriptor.source_document_set_digest()
        );
        assert_eq!(
            field_u32(row, 5),
            descriptor.source_document_set_digest_version()
        );
        assert_eq!(field_u32(row, 6), descriptor.frontend_version());
        assert_eq!(field_sha256(row, 7), *descriptor.frontend_options_digest());
        assert_eq!(field_utf8(row, 8), descriptor.generator_build_id());
        assert_eq!(
            field_sha256(row, 9),
            *descriptor.parameters_and_inputs_digest()
        );
        assert_eq!(
            row.field_by_tag(10)
                .map(|field| match field.value().unwrap() {
                    RegistryCheckedFieldValue::U64(value) => value,
                    value => panic!("expected random seed, got {value:?}"),
                }),
            descriptor.random_seed()
        );
        assert_eq!(field_utf8(row, 11), descriptor.provenance());
        let imports = match row.field_by_tag(12).unwrap().value().unwrap() {
            RegistryCheckedFieldValue::RecordVector(imports) => (0..imports.len())
                .map(|index| field_utf8(imports.row(index).unwrap(), 1).to_owned())
                .collect::<Vec<_>>(),
            value => panic!("expected imports, got {value:?}"),
        };
        assert_eq!(
            imports,
            descriptor.imports().map(str::to_owned).collect::<Vec<_>>()
        );
    }

    let mut documents = output
        .source_map_input()
        .source_documents()
        .map(|document| {
            (
                *module_ordinals
                    .get(document.authoring_namespace_id())
                    .unwrap(),
                document.source_document_key(),
                document,
            )
        })
        .collect::<Vec<_>>();
    documents.sort_unstable_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.as_bytes().cmp(right.1.as_bytes()))
    });
    assert_eq!(
        document_table.row_count(),
        u32::try_from(documents.len()).unwrap()
    );
    for (ordinal, (module, _, document)) in documents.iter().enumerate() {
        let row = document_table.row(u32::try_from(ordinal).unwrap()).unwrap();
        assert_eq!(field_u32(row, 1), u32::try_from(ordinal).unwrap());
        assert_eq!(field_u32(row, 2), *module);
        assert_eq!(field_utf8(row, 3), document.source_document_key());
        assert_eq!(field_sha256(row, 4), *document.source_document_digest());
        assert_eq!(field_u32(row, 5), document.source_record_byte_len());
        assert_eq!(
            row.field_by_tag(6)
                .map(|field| match field.value().unwrap() {
                    RegistryCheckedFieldValue::Utf8(value) => value,
                    value => panic!("expected display source, got {value:?}"),
                }),
            document.origin().display_source()
        );
    }

    let location_keys = (0..location_table.row_count())
        .map(|ordinal| {
            let row = location_table.row(ordinal).unwrap();
            assert_eq!(field_u32(row, 1), ordinal);
            let source_module_ordinal = field_u32(row, 3);
            let source_language = module_views[usize::try_from(source_module_ordinal).unwrap()]
                .descriptor()
                .source_language();
            assert_eq!(
                field_u8(row, 2),
                u8::from(matches!(
                    source_language,
                    crate::SourceLanguage::RoadEditingSource
                )),
                "location kind follows its source module language"
            );
            row.fields()
                .filter(|field| field.tag() != 1)
                .map(|field| (field.tag(), field.value_bytes().to_vec()))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        location_keys.iter().collect::<BTreeSet<_>>().len(),
        location_keys.len()
    );

    let identity_table = artifact.section(1).unwrap().table(0).unwrap();
    let identity_keys = (0..identity_table.row_count())
        .map(|ordinal| {
            let row = identity_table.row(ordinal).unwrap();
            (
                field_u16(row, 1),
                field_stable_id(row, 3),
                field_u32(row, 2),
            )
        })
        .collect::<BTreeSet<_>>();
    let stable_table = source_map.section(2).unwrap().table(0).unwrap();
    let stable_keys = (0..stable_table.row_count())
        .map(|ordinal| {
            let row = stable_table.row(ordinal).unwrap();
            assert!(field_ordinals(row, 5).is_empty());
            (
                field_u16(row, 1),
                field_stable_id(row, 2),
                field_u32(row, 3),
            )
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(stable_keys, identity_keys);

    let owner_table = source_map.section(3).unwrap().table(0).unwrap();
    let owner_keys = (0..owner_table.row_count())
        .map(|ordinal| {
            let row = owner_table.row(ordinal).unwrap();
            (
                field_u16(row, 1),
                field_stable_id(row, 2),
                field_u8(row, 3),
                field_u32(row, 4),
            )
        })
        .collect::<Vec<_>>();
    assert!(owner_keys.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(
        owner_keys.iter().copied().collect::<BTreeSet<_>>().len(),
        owner_keys.len()
    );

    let derived_table = source_map.section(4).unwrap().table(0).unwrap();
    let derived_keys = (0..derived_table.row_count())
        .map(|ordinal| {
            let row = derived_table.row(ordinal).unwrap();
            (
                field_u16(row, 1),
                field_stable_id(row, 2),
                field_u8(row, 3),
                field_u32(row, 4),
            )
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        derived_keys,
        owner_keys
            .iter()
            .copied()
            .filter(|key| key.2 == 9)
            .collect()
    );

    let mut referenced_locations = BTreeSet::new();
    for ordinal in 0..module_table.row_count() {
        referenced_locations.insert(field_u32(module_table.row(ordinal).unwrap(), 13));
    }
    for ordinal in 0..stable_table.row_count() {
        let row = stable_table.row(ordinal).unwrap();
        referenced_locations.insert(field_u32(row, 4));
        referenced_locations.extend(field_ordinals(row, 5));
    }
    for ordinal in 0..owner_table.row_count() {
        let row = owner_table.row(ordinal).unwrap();
        referenced_locations.insert(field_u32(row, 5));
        referenced_locations.extend(field_ordinals(row, 6));
    }
    let spatial_range_table = source_map.section(3).unwrap().table(1).unwrap();
    for ordinal in 0..spatial_range_table.row_count() {
        referenced_locations.insert(field_u32(spatial_range_table.row(ordinal).unwrap(), 8));
    }
    for ordinal in 0..derived_table.row_count() {
        referenced_locations.extend(field_ordinals(derived_table.row(ordinal).unwrap(), 7));
    }
    assert_eq!(
        referenced_locations,
        (0..location_table.row_count()).collect::<BTreeSet<_>>()
    );

    assert!(
        (0..spatial_range_table.row_count())
            .any(|ordinal| { field_u8(spatial_range_table.row(ordinal).unwrap(), 3) == 32 })
    );
    assert!(
        (0..owner_table.row_count())
            .any(|ordinal| field_u8(owner_table.row(ordinal).unwrap(), 3) == 32)
    );
    assert!((0..owner_table.row_count()).any(|ordinal| {
        let row = owner_table.row(ordinal).unwrap();
        field_u8(row, 3) == 28 && !field_ordinals(row, 6).is_empty()
    }));
    for ordinal in 0..owner_table.row_count() {
        let row = owner_table.row(ordinal).unwrap();
        if field_u8(row, 3) == 29 {
            assert!(field_ordinals(row, 6).is_empty());
        }
    }
    let spatial = artifact.section(4).unwrap();
    let lane_geometry = spatial.table(1).unwrap();
    let direction_profile_flags = (0..lane_geometry.row_count())
        .map(|ordinal| field_u8(lane_geometry.row(ordinal).unwrap(), 6))
        .collect::<BTreeSet<_>>();
    assert_eq!(direction_profile_flags, BTreeSet::from([0, 1]));
    for ordinal in 0..spatial.table(2).unwrap().row_count() {
        assert_eq!(
            field_u8(spatial.table(2).unwrap().row(ordinal).unwrap(), 4),
            0
        );
    }
}

#[test]
fn map_direct_versions_languages_document_lengths_locations_and_derived_values_fail_closed() {
    let mutate_u16 = |section, table, row, tag, value: u16| {
        let mut bytes = FULL_SPATIAL_EXPECTED_LFSM.to_vec();
        let range = field_value_range(
            &bytes,
            PortableObjectKind::SourceMap,
            section,
            table,
            row,
            tag,
        );
        let changed_at = range.start;
        bytes[range].copy_from_slice(&value.to_le_bytes());
        refresh_chunk_digest_containing(&mut bytes, PortableObjectKind::SourceMap, changed_at);
        bytes
    };
    let mutate_u32 = |section, table, row, tag, value: u32| {
        let mut bytes = FULL_SPATIAL_EXPECTED_LFSM.to_vec();
        let range = field_value_range(
            &bytes,
            PortableObjectKind::SourceMap,
            section,
            table,
            row,
            tag,
        );
        let changed_at = range.start;
        bytes[range].copy_from_slice(&value.to_le_bytes());
        refresh_chunk_digest_containing(&mut bytes, PortableObjectKind::SourceMap, changed_at);
        bytes
    };
    for bytes in [
        mutate_u16(0, 0, 0, 1, 4),
        mutate_u16(0, 0, 0, 3, 1),
        mutate_u16(0, 0, 0, 7, 2),
    ] {
        assert_eq!(
            preflight_object_values(&bytes, PortableObjectKind::SourceMap, FormatLimits::HARD,)
                .unwrap_err()
                .class(),
            FormatErrorClass::NonCanonicalValue
        );
    }
    assert_eq!(
        preflight_object_values(
            &mutate_u16(1, 0, 0, 3, 3),
            PortableObjectKind::SourceMap,
            FormatLimits::HARD,
        )
        .unwrap_err()
        .class(),
        FormatErrorClass::UnknownKind
    );
    for bytes in [mutate_u32(1, 1, 0, 5, 0), mutate_u32(1, 2, 0, 5, 0)] {
        assert_eq!(
            preflight_object_values(&bytes, PortableObjectKind::SourceMap, FormatLimits::HARD,)
                .unwrap_err()
                .class(),
            FormatErrorClass::NonCanonicalValue
        );
    }
    let derived = registry(FULL_SPATIAL_EXPECTED_LFSM, PortableObjectKind::SourceMap)
        .section(4)
        .unwrap()
        .table(0)
        .unwrap();
    assert!(derived.row_count() > 0);
    for tag in [5, 6] {
        let bytes = mutate_u16(4, 0, 0, tag, 3);
        assert_eq!(
            preflight_object_values(&bytes, PortableObjectKind::SourceMap, FormatLimits::HARD,)
                .unwrap_err()
                .class(),
            FormatErrorClass::NonCanonicalValue
        );
    }
}
