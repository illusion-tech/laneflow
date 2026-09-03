use super::*;
use crate::{CompileLimits, check_portable_policy_diff};

fn root(object: &OwnedObject) -> Box<[u8]> {
    let mut object = object.clone();
    let revision = network_revision(&raw_bytes(&object), FormatLimits::HARD).unwrap();
    set_lfca_network_revision(&mut object, revision).unwrap();
    raw_bytes(&object)
}

fn generate(base: &[u8], target: &[u8]) -> OwnedObject {
    let base = preflight_object_values(
        base,
        PortableObjectKind::CanonicalArtifact,
        FormatLimits::HARD,
    )
    .unwrap();
    let revision = network_revision(target, FormatLimits::HARD).unwrap();
    build_artifact_lfsd(
        base,
        revision,
        &close_object(target.into()),
        FormatLimits::HARD,
        u64::MAX,
    )
    .unwrap()
    .0
}

fn verify(base: &[u8], target: &[u8], diff: &OwnedObject) -> Result<(), PortableEmissionError> {
    let base = preflight_object_values(
        base,
        PortableObjectKind::CanonicalArtifact,
        FormatLimits::HARD,
    )?;
    check_portable_policy_diff(
        PortableDiffBase::Artifact(base),
        target,
        &raw_bytes(diff),
        FormatLimits::HARD,
        &CompileLimits::single_network_1m_v2(),
    )
}

fn changed_fixture() -> OwnedObject {
    let mut changed = fixture();
    replace_field(
        &mut changed,
        1,
        3,
        OwnedValue::Utf8("fixture:changed-source".into()),
    );
    replace_field(&mut changed, 2, 4, OwnedValue::U64(500));
    replace_field(&mut changed, 3, 5, OwnedValue::I32(12));
    replace_field(&mut changed, 4, 6, OwnedValue::U8(1));
    changed
}

fn without_policies() -> OwnedObject {
    let mut object = fixture();
    object.sections[1].tables[0].rows = object.sections[1].tables[0]
        .rows
        .iter()
        .filter(|r| r.fields[0].value != OwnedValue::U16(24))
        .cloned()
        .collect();
    object.sections[2].tables[23].rows = Box::new([]);
    for table in 1..=4 {
        object.sections[3].tables[table].rows = Box::new([]);
    }
    object
}

#[test]
fn policy_diff_add_remove_modify_and_genesis_include_every_member() {
    let base = root(&fixture());
    let target = root(&changed_fixture());
    let empty = root(&without_policies());
    for (b, a, op) in [(&base, &target, 2), (&empty, &base, 0), (&base, &empty, 1)] {
        let diff = generate(b, a);
        assert_eq!(diff.sections[6].tables[0].rows.len(), 4);
        for (kind, row) in diff.sections[6].tables[0].rows.iter().enumerate() {
            assert_eq!(row.fields[0].value, OwnedValue::U8(op));
            assert_eq!(row.fields[2].value, OwnedValue::U8(kind as u8));
        }
        verify(b, a, &diff).unwrap();
        let mut omitted = diff.clone();
        omitted.sections[6].tables[0].rows = Box::new([]);
        assert_eq!(
            verify(b, a, &omitted),
            Err(PortableEmissionError::PolicyDiffMismatch)
        );
        if op != 2 {
            let mut missing_entity = diff.clone();
            missing_entity.sections[1].tables[0].rows = missing_entity.sections[1].tables[0]
                .rows
                .iter()
                .filter(|r| r.fields[1].value != OwnedValue::U16(24))
                .cloned()
                .collect();
            assert_eq!(
                verify(b, a, &missing_entity),
                Err(PortableEmissionError::PolicyDiffMismatch)
            );
        }
    }
    let output = crate::compiler::portable_fixture_tests::full_spatial_portable_fixture_output();
    let genesis = build_genesis_lfsd(
        &output,
        network_revision(&base, FormatLimits::HARD).unwrap(),
        &close_object(base.clone()),
        FormatLimits::HARD,
    )
    .unwrap();
    check_portable_policy_diff(
        PortableDiffBase::Genesis,
        &base,
        &raw_bytes(&genesis),
        FormatLimits::HARD,
        output.compile_limits(),
    )
    .unwrap();
    assert_eq!(genesis.sections[6].tables[0].rows.len(), 4);
}

#[test]
fn policy_diff_checker_rejects_omission_extra_wrong_owner_kind_side_and_payload() {
    let base = root(&fixture());
    let target = root(&changed_fixture());
    let good = generate(&base, &target);
    for mutation in 0..7 {
        let mut bad = good.clone();
        let rows = &mut bad.sections[6].tables[0].rows;
        match mutation {
            0 => *rows = rows[1..].into(),
            1 => rows[0].fields[1].value = OwnedValue::StableId128([0x42; 16]),
            2 => rows[0].fields[3].value = OwnedValue::Utf8("missing".into()),
            3 => {
                let row = &mut rows[0];
                row.fields.swap(4, 5);
                row.fields[4].tag = 5;
                row.fields[5].tag = 6;
            }
            4 => rows[0].fields[5].value = rows[0].fields[4].value.clone(),
            5 => rows[0].fields[2].value = OwnedValue::U8(33),
            6 => {
                let OwnedValue::Bytes(value) = &mut rows[3].fields[5].value else {
                    panic!()
                };
                *value.last_mut().unwrap() = b'b';
            }
            _ => unreachable!(),
        }
        // unknown member schema 可在 writer 的结构预检更早拒绝。
        if mutation == 5 {
            assert!(encode_owned_object(&bad, FormatLimits::HARD, None).is_err());
        } else {
            assert!(verify(&base, &target, &bad).is_err(), "mutation {mutation}");
        }
    }
    let mut split = good.clone();
    let modified = split.sections[6].tables[0].rows[0].clone();
    let mut add = modified.clone();
    add.fields = [
        modified.fields[0].clone(),
        modified.fields[1].clone(),
        modified.fields[2].clone(),
        modified.fields[3].clone(),
        modified.fields[5].clone(),
    ]
    .into();
    add.fields[0].value = OwnedValue::U8(0);
    let mut remove = modified.clone();
    remove.fields = modified.fields[..5].into();
    remove.fields[0].value = OwnedValue::U8(1);
    split.sections[6].tables[0].rows = [
        vec![add, remove],
        good.sections[6].tables[0].rows[1..].to_vec(),
    ]
    .concat()
    .into();
    assert!(verify(&base, &target, &split).is_err());
    let mut missing_section = good.clone();
    missing_section.sections = good.sections[..6].into();
    assert!(encode_owned_object(&missing_section, FormatLimits::HARD, None).is_err());
    verify(&base, &target, &good).unwrap();
}

fn reorder_kind(object: &mut OwnedObject, kind: u16) {
    object.sections[2].tables[usize::from(kind - 1)]
        .rows
        .swap(0, 1);
    for (i, row) in object.sections[2].tables[usize::from(kind - 1)]
        .rows
        .iter_mut()
        .enumerate()
    {
        row.fields[0].value = OwnedValue::U32(i as u32);
    }
    let ids = &mut object.sections[1].tables[0].rows;
    let start = ids
        .iter()
        .position(|r| r.fields[0].value == OwnedValue::U16(kind))
        .unwrap();
    ids.swap(start, start + 1);
    ids[start].fields[1].value = OwnedValue::U32(0);
    ids[start + 1].fields[1].value = OwnedValue::U32(1);
}

#[test]
fn policy_diff_uses_stable_identity_across_ordinal_reordering_and_renames() {
    let base = root(&fixture());
    let mut target_object = fixture();
    reorder_kind(&mut target_object, 24);
    for table in 1..=4 {
        replace_field(&mut target_object, table, 1, OwnedValue::U32(1));
    }
    reorder_kind(&mut target_object, 18);
    for table in [3, 4] {
        replace_field(
            &mut target_object,
            table,
            4,
            OwnedValue::OrdinalVectorU32([0, 1].into()),
        );
    }
    let target = root(&target_object);
    let no_local_change = generate(&base, &target);
    assert!(no_local_change.sections[6].tables[0].rows.is_empty());
    verify(&base, &target, &no_local_change).unwrap();
    replace_field(
        &mut target_object,
        3,
        2,
        OwnedValue::Utf8("renamed-rule".into()),
    );
    let target = root(&target_object);
    let renamed = generate(&base, &target);
    assert_eq!(renamed.sections[6].tables[0].rows.len(), 2);
    assert_eq!(
        renamed.sections[6].tables[0].rows[0].fields[0].value,
        OwnedValue::U8(0)
    );
    assert_eq!(
        renamed.sections[6].tables[0].rows[1].fields[0].value,
        OwnedValue::U8(1)
    );
    verify(&base, &target, &renamed).unwrap();
}

#[test]
fn policy_diff_preserves_entity_attribute_partition_and_checks_exact_roots() {
    let base = root(&fixture());
    let mut object = fixture();
    object.sections[2].tables[23].rows[0].fields[3].value = OwnedValue::Utf8("policy-v2".into());
    let mut movement = object.sections[2].tables[5].rows[0].fields.to_vec();
    movement.push(field(7, OwnedValue::U8(1)));
    object.sections[2].tables[5].rows[0].fields = movement.into();
    let target = root(&object);
    let diff = generate(&base, &target);
    verify(&base, &target, &diff).unwrap();
    assert!(diff.sections[6].tables[0].rows.is_empty());
    for section in [1, 4] {
        let mut bad = diff.clone();
        bad.sections[section].tables[0].rows = Box::new([]);
        assert_eq!(
            verify(&base, &target, &bad),
            Err(PortableEmissionError::PolicyDiffMismatch)
        );
    }
    assert!(verify(&target, &base, &diff).is_err());
    let mut bad_root = target.to_vec();
    *bad_root.last_mut().unwrap() ^= 1;
    assert!(verify(&base, &bad_root, &diff).is_err());
    let mut old = raw_bytes(&diff).to_vec();
    old[4..6].copy_from_slice(&3_u16.to_le_bytes());
    assert!(
        preflight_object_values(&old, PortableObjectKind::SemanticDiff, FormatLimits::HARD)
            .is_err()
    );
}

#[test]
fn policy_diff_cross_chunk_members_and_cross_operation_duplicate_key() {
    let base = root(&without_policies());
    let mut object = fixture();
    let mut evidence = object.sections[3].tables[1].rows.to_vec();
    evidence.extend((0..65_536).map(|i| {
        row([
            field(1, OwnedValue::U32(0)),
            field(2, OwnedValue::Utf8(format!("key-{i:05}").into())),
            field(3, OwnedValue::Utf8("fixture:source".into())),
        ])
    }));
    object.sections[3].tables[1].rows = evidence.into();
    let target = root(&object);
    let diff = generate(&base, &target);
    let bytes = raw_bytes(&diff);
    let checked =
        preflight_object_values(&bytes, PortableObjectKind::SemanticDiff, FormatLimits::HARD)
            .unwrap();
    assert_eq!(
        checked
            .registry_view()
            .section(6)
            .unwrap()
            .table(0)
            .unwrap()
            .chunk_count(),
        2
    );
    verify(&base, &target, &diff).unwrap();
    let mut duplicate = diff.clone();
    let mut rows = duplicate.sections[6].tables[0].rows.to_vec();
    let mut remove = rows[0].clone();
    remove.fields[0].value = OwnedValue::U8(1);
    remove.fields[4].tag = 5;
    rows.push(remove);
    duplicate.sections[6].tables[0].rows = rows.into();
    assert!(verify(&base, &target, &duplicate).is_err());
}

#[test]
fn policy_diff_scratch_and_embedded_budgets_fail_before_output_and_retry() {
    let base = root(&fixture());
    let target = root(&changed_fixture());
    let diff = generate(&base, &target);
    let base_view = preflight_object_values(
        &base,
        PortableObjectKind::CanonicalArtifact,
        FormatLimits::HARD,
    )
    .unwrap();
    let tiny = CompileLimits::single_network_1m_v2()
        // 索引本身也必须在首次分配前受同一个预算约束。
        .with_test_admission_limit(CompileLimitDimension::StageScratchBytes, 64);
    assert!(matches!(
        check_portable_policy_diff(
            PortableDiffBase::Artifact(base_view),
            &target,
            &raw_bytes(&diff),
            FormatLimits::HARD,
            &tiny
        ),
        Err(PortableEmissionError::CompileLimitExceeded {
            dimension: CompileLimitDimension::StageScratchBytes,
            ..
        })
    ));
    assert!(matches!(
        build_artifact_lfsd(
            base_view,
            network_revision(&target, FormatLimits::HARD).unwrap(),
            &close_object(target.clone()),
            FormatLimits::HARD,
            64
        ),
        Err(PortableEmissionError::CompileLimitExceeded { .. })
    ));
    for vector in [false, true] {
        let mut config = laneflow_format::FormatLimitConfig::HARD;
        if vector {
            config.max_total_vector_bytes = 0;
        } else {
            config.max_utf8_field_bytes = 8;
        }
        let limits = FormatLimits::try_new(config).unwrap();
        assert!(encode_owned_object(&diff, limits, None).is_err());
        assert!(
            preflight_object_values(&raw_bytes(&diff), PortableObjectKind::SemanticDiff, limits)
                .is_err()
        );
    }
    verify(&base, &target, &diff).unwrap();
}

#[test]
fn policy_diff_counts_both_artifact_indexes_in_one_scratch_budget() {
    let base = root(&fixture());
    let target = root(&changed_fixture());
    let diff = generate(&base, &target);
    let target_view = preflight_object_values(
        &target,
        PortableObjectKind::CanonicalArtifact,
        FormatLimits::HARD,
    )
    .unwrap();
    let base_view = preflight_object_values(
        &base,
        PortableObjectKind::CanonicalArtifact,
        FormatLimits::HARD,
    )
    .unwrap();
    let mut measured = Scratch::new(u64::MAX);
    let _target_index =
        ArtifactIndex::build(target_view.registry_view(), MISMATCH, &mut measured).unwrap();
    let one_index = measured.used();
    let _base_index =
        ArtifactIndex::build(base_view.registry_view(), MISMATCH, &mut measured).unwrap();
    assert!(measured.used() > one_index);
    let limit = CompileLimits::single_network_1m_v2().with_test_admission_limit(
        CompileLimitDimension::StageScratchBytes,
        one_index.try_into().unwrap(),
    );
    assert!(
        matches!(check_portable_policy_diff(PortableDiffBase::Artifact(base_view), &target, &raw_bytes(&diff), FormatLimits::HARD, &limit),
        Err(PortableEmissionError::CompileLimitExceeded { dimension: CompileLimitDimension::StageScratchBytes, actual, limit }) if actual > limit && limit == one_index)
    );
    // 预算失败不污染下一次同输入检查。
    verify(&base, &target, &diff).unwrap();
}

#[test]
fn policy_diff_fixed_bytes_cover_all_four_complete_member_values() {
    let base = root(&fixture());
    let target = root(&changed_fixture());
    let diff = generate(&base, &target);
    let bytes = raw_bytes(&diff);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/portable/lfsd-policy-members");
    if std::env::var_os("DUMP_PORTABLE").is_some() {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("base.lfca"), &base).unwrap();
        std::fs::write(dir.join("target.lfca"), &target).unwrap();
        std::fs::write(dir.join("expected.lfsd"), &bytes).unwrap();
        std::fs::write(
            dir.join("bindings.txt"),
            format!(
                "length={}\nkey={}\n",
                bytes.len(),
                object_key(sha256(&bytes))
            ),
        )
        .unwrap();
        return;
    }
    assert_eq!(
        bytes.as_ref(),
        include_bytes!("../../../../../tests/fixtures/portable/lfsd-policy-members/expected.lfsd")
    );
    assert_eq!(bytes.len(), 2_266);
    assert_eq!(
        object_key(sha256(&bytes)).as_ref(),
        "sha256/f2dfe802d21bf001aac3850a5a8aa6d306a6ad19388e5cc16f704f2f9046f4ca"
    );
    verify(&base, &target, &diff).unwrap();
}

#[test]
fn policy_value_strings_determine_canonical_chunk_boundaries() {
    // 每侧 2 MiB 字符串；两行的内层 UTF-8 总量加外层 key 超过 8 MiB，必须分块。
    fn evidence_value(first: u8) -> Box<[u8]> {
        let size = 1_048_576;
        let mut bytes = Vec::with_capacity(16 + 2 * (12 + size));
        bytes.extend_from_slice(&((16 + 2 * (12 + size)) as u64).to_le_bytes());
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        for tag in [3_u16, 4] {
            bytes.extend_from_slice(&tag.to_le_bytes());
            bytes.extend_from_slice(&[9, 0]);
            bytes.extend_from_slice(&(size as u64).to_le_bytes());
            bytes.resize(bytes.len() + size, b'a');
        }
        bytes[28] = first;
        bytes.into()
    }
    let mut diff = generate(&root(&fixture()), &root(&changed_fixture()));
    let template = diff.sections[6].tables[0].rows[0].clone();
    diff.sections[6].tables[0].rows = (0..3)
        .map(|i| {
            let mut row = template.clone();
            row.fields[3].value = OwnedValue::Utf8(format!("evidence-{i}").into());
            row.fields[4].value = OwnedValue::Bytes(evidence_value(b'a'));
            row.fields[5].value = OwnedValue::Bytes(evidence_value(b'b'));
            row
        })
        .collect();
    let bytes = raw_bytes(&diff);
    let checked =
        preflight_object_values(&bytes, PortableObjectKind::SemanticDiff, FormatLimits::HARD)
            .unwrap();
    assert_eq!(
        checked
            .registry_view()
            .section(6)
            .unwrap()
            .table(0)
            .unwrap()
            .chunk_count(),
        3
    );
}
