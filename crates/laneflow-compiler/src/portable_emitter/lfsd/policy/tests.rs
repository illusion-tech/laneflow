use super::*;
mod diff;

const FULL_LFCA: &[u8] =
    include_bytes!("../../../../tests/fixtures/portable/lfca-full-spatial/expected.lfca");
const MISMATCH: PortableEmissionError = PortableEmissionError::DiffBaseSemanticMismatch;

fn own_row(input: RegistryCheckedRowView<'_>) -> OwnedRow {
    row(input.fields().map(|input| {
        let value = match input.value().unwrap() {
            RegistryCheckedFieldValue::U8(v) => OwnedValue::U8(v),
            RegistryCheckedFieldValue::U16(v) => OwnedValue::U16(v),
            RegistryCheckedFieldValue::U32(v) => OwnedValue::U32(v),
            RegistryCheckedFieldValue::U64(v) => OwnedValue::U64(v),
            RegistryCheckedFieldValue::I32(v) => OwnedValue::I32(v),
            RegistryCheckedFieldValue::F32(v) => OwnedValue::F32(v),
            RegistryCheckedFieldValue::F64(v) => OwnedValue::F64(v),
            RegistryCheckedFieldValue::StableId128(v) => OwnedValue::StableId128(v.into_bytes()),
            RegistryCheckedFieldValue::Sha256(v) => OwnedValue::Sha256(v.into_bytes()),
            RegistryCheckedFieldValue::Utf8(v) => OwnedValue::Utf8(v.into()),
            RegistryCheckedFieldValue::Bytes(v) => OwnedValue::Bytes(v.into()),
            RegistryCheckedFieldValue::OrdinalVectorU32(v) => {
                OwnedValue::OrdinalVectorU32((0..v.len()).map(|i| v.get(i).unwrap()).collect())
            }
            RegistryCheckedFieldValue::RecordVector(v) => {
                OwnedValue::RecordVector(v.rows().map(own_row).collect())
            }
        };
        field(input.tag(), value)
    }))
}

fn stable_order(object: &OwnedObject, kind: u16) -> Box<[u32]> {
    let mut entries = object.sections[2].tables[usize::from(kind - 1)]
        .rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let OwnedValue::StableId128(id) = row.fields[1].value else {
                panic!("stable ID")
            };
            (id, i as u32)
        })
        .collect::<Vec<_>>();
    entries.sort_unstable();
    entries.into_iter().map(|(_, ordinal)| ordinal).collect()
}

fn evidence_keys(keys: &[&str]) -> OwnedValue {
    OwnedValue::RecordVector(
        keys.iter()
            .map(|key| row([field(1, OwnedValue::Utf8((*key).into()))]))
            .collect(),
    )
}

fn fixture() -> OwnedObject {
    let input = preflight_object_values(
        FULL_LFCA,
        PortableObjectKind::CanonicalArtifact,
        FormatLimits::HARD,
    )
    .unwrap()
    .registry_view();
    let mut object = OwnedObject {
        kind: PortableObjectKind::CanonicalArtifact,
        sections: input
            .sections()
            .map(|s| {
                section(
                    s.kind(),
                    s.tables().map(|t| table(t.kind(), t.rows().map(own_row))),
                )
            })
            .collect(),
    };
    let mut identities = object.sections[1].tables[0].rows.to_vec();
    let mut policies = Vec::new();
    for (ordinal, key) in [(0, "a"), (1, "b")] {
        let id = crate::derive_canonical_stable_id_v1(
            EntityKind::RightOfWayPolicySet,
            "fixture/policy",
            key,
            &crate::CompileLimits::single_network_1m_v2(),
        )
        .unwrap()
        .into_bytes();
        identities.push(row([
            field(1, OwnedValue::U16(24)),
            field(2, OwnedValue::U32(ordinal)),
            field(3, OwnedValue::StableId128(id)),
            field(
                4,
                OwnedValue::RecordVector(
                    [
                        row([
                            field(1, OwnedValue::U16(1)),
                            field(
                                2,
                                OwnedValue::Bytes(b"fixture/policy".to_vec().into_boxed_slice()),
                            ),
                        ]),
                        row([
                            field(1, OwnedValue::U16(35)),
                            field(2, OwnedValue::Bytes(key.as_bytes().into())),
                        ]),
                    ]
                    .into(),
                ),
            ),
        ]));
        policies.push(row([
            field(1, OwnedValue::U32(ordinal)),
            field(2, OwnedValue::StableId128(id)),
            field(3, OwnedValue::Utf8("engineering-fixture".into())),
            field(4, OwnedValue::Utf8("policy-v1".into())),
            field(5, OwnedValue::Utf8("fixture:policy-references-v1".into())),
        ]));
    }
    object.sections[1].tables[0].rows = identities.into_boxed_slice();
    object.sections[2].tables[23].rows = policies.into_boxed_slice();
    object.sections[3].tables[1].rows = [row([
        field(1, OwnedValue::U32(0)),
        field(2, OwnedValue::Utf8("a".into())),
        field(3, OwnedValue::Utf8("fixture:rule-source".into())),
    ])]
    .into();
    object.sections[3].tables[2].rows = [row([
        field(1, OwnedValue::U32(0)),
        field(2, OwnedValue::Utf8("a".into())),
        field(3, OwnedValue::Utf8("gap-v1".into())),
        field(4, OwnedValue::U64(0)),
        field(5, OwnedValue::U64(1)),
        field(6, OwnedValue::U64(u64::MAX)),
    ])]
    .into();
    let classes = stable_order(&object, 18);
    let streams = stable_order(&object, 23);
    assert!(streams.len() >= 2);
    object.sections[3].tables[3].rows = [row([
        field(1, OwnedValue::U32(0)),
        field(2, OwnedValue::Utf8("rule".into())),
        field(3, OwnedValue::U32(0)),
        field(4, OwnedValue::OrdinalVectorU32(classes.clone())),
        field(5, OwnedValue::I32(-1)),
        field(6, OwnedValue::OrdinalVectorU32(streams)),
        field(7, OwnedValue::Utf8("a".into())),
        field(8, evidence_keys(&["a"])),
    ])]
    .into();
    object.sections[3].tables[4].rows = [row([
        field(1, OwnedValue::U32(0)),
        field(2, OwnedValue::Utf8("rule".into())),
        field(3, OwnedValue::U32(0)),
        field(4, OwnedValue::OrdinalVectorU32(classes)),
        field(5, OwnedValue::U8(0)),
        field(6, OwnedValue::U8(0)),
        field(7, evidence_keys(&["a"])),
    ])]
    .into();
    object
}

fn raw_bytes(object: &OwnedObject) -> Box<[u8]> {
    encode_owned_object(object, FormatLimits::HARD, None).unwrap()
}

fn check(object: &OwnedObject, scratch_limit: u64) -> Result<(), PortableEmissionError> {
    let bytes = raw_bytes(object);
    let view = preflight_object_values(
        &bytes,
        PortableObjectKind::CanonicalArtifact,
        FormatLimits::HARD,
    )?
    .registry_view();
    let mut scratch = Scratch::new(scratch_limit);
    let index = ArtifactIndex::build(view, MISMATCH, &mut scratch)?;
    validate_policy_references(&index, &mut scratch, MISMATCH)
}

fn replace_field(object: &mut OwnedObject, table: usize, tag: u16, value: OwnedValue) {
    object.sections[3].tables[table].rows[0]
        .fields
        .iter_mut()
        .find(|f| f.tag == tag)
        .unwrap()
        .value = value;
}

#[test]
fn policy_references_resolve_same_owner_keys_and_stable_identity_sets() {
    let object = fixture();
    // StableId 的规范顺序与 typed ordinal 顺序不同，不能直接排序 ordinal。
    assert_eq!(stable_order(&object, 18).as_ref(), &[1, 0]);
    check(&object, u64::MAX).unwrap();
    // Evidence 与 GapProfile 可同名；两个 policy 的局部命名空间彼此独立。
    let mut shared_names = object.clone();
    for table in [1, 2] {
        let mut members = shared_names.sections[3].tables[table].rows.to_vec();
        let mut other = members[0].clone();
        other.fields[0].value = OwnedValue::U32(1);
        members.push(other);
        shared_names.sections[3].tables[table].rows = members.into_boxed_slice();
    }
    check(&shared_names, u64::MAX).unwrap();
    for table in 1..=4 {
        let mut bad = object.clone();
        replace_field(&mut bad, table, 1, OwnedValue::U32(2));
        assert_eq!(check(&bad, u64::MAX), Err(MISMATCH), "table {table} owner");
    }
    for (table, tag) in [(3, 3), (4, 3), (3, 4), (4, 4), (3, 6)] {
        let mut bad = object.clone();
        let value = if tag == 3 {
            OwnedValue::U32(u32::MAX)
        } else {
            OwnedValue::OrdinalVectorU32([u32::MAX].into())
        };
        replace_field(&mut bad, table, tag, value);
        assert_eq!(
            check(&bad, u64::MAX),
            Err(MISMATCH),
            "table {table} tag {tag}"
        );
    }
    for table in [1, 2] {
        let mut bad = object.clone();
        replace_field(&mut bad, table, 1, OwnedValue::U32(1));
        assert_eq!(
            check(&bad, u64::MAX),
            Err(MISMATCH),
            "foreign local key table {table}"
        );
    }
    for (table, tag) in [(3, 4), (4, 4), (3, 6)] {
        let mut bad = object.clone();
        replace_field(
            &mut bad,
            table,
            tag,
            OwnedValue::OrdinalVectorU32([0, 0].into()),
        );
        assert_eq!(check(&bad, u64::MAX), Err(MISMATCH));
    }
    let mut bad = object.clone();
    let mut reversed = stable_order(&object, 23);
    reversed.reverse();
    replace_field(&mut bad, 3, 6, OwnedValue::OrdinalVectorU32(reversed));
    assert_eq!(check(&bad, u64::MAX), Err(MISMATCH));
    check(&object, u64::MAX).unwrap();
}

#[test]
fn every_rule_requires_inherited_or_resolved_evidence() {
    for (table, evidence_tag) in [(3, 8), (4, 7)] {
        let mut object = fixture();
        replace_field(&mut object, table, evidence_tag, evidence_keys(&[]));
        check(&object, u64::MAX).unwrap();
        object.sections[2].tables[23].rows[0].fields =
            object.sections[2].tables[23].rows[0].fields[..4].into();
        assert_eq!(check(&object, u64::MAX), Err(MISMATCH));
        replace_field(&mut object, table, evidence_tag, evidence_keys(&["a"]));
        check(&object, u64::MAX).unwrap();
        replace_field(
            &mut object,
            table,
            evidence_tag,
            evidence_keys(&["missing"]),
        );
        assert_eq!(check(&object, u64::MAX), Err(MISMATCH));
        object.sections[2].tables[23].rows[0] = fixture().sections[2].tables[23].rows[0].clone();
        assert_eq!(check(&object, u64::MAX), Err(MISMATCH));
    }
}

#[test]
fn local_reference_scratch_is_measured_before_allocation_and_retry_is_clean() {
    let object = fixture();
    let encoded = raw_bytes(&object);
    let view = preflight_object_values(
        &encoded,
        PortableObjectKind::CanonicalArtifact,
        FormatLimits::HARD,
    )
    .unwrap()
    .registry_view();
    let mut scratch = Scratch::new(u64::MAX);
    let _index = ArtifactIndex::build(view, MISMATCH, &mut scratch).unwrap();
    let exact = scratch.used() + 2 * core::mem::size_of::<LocalKey<'_>>() as u64;
    assert_eq!(
        check(&object, exact - 1),
        Err(PortableEmissionError::CompileLimitExceeded {
            dimension: CompileLimitDimension::StageScratchBytes,
            actual: exact,
            limit: exact - 1,
        })
    );
    check(&object, exact).unwrap();
}

#[test]
fn evidence_reference_and_duplicate_detection_cross_chunk_boundaries() {
    let mut object = fixture();
    object.sections[3].tables[1].rows = (0..65_537)
        .map(|i| {
            row([
                field(1, OwnedValue::U32(0)),
                field(2, OwnedValue::Utf8(format!("key-{i:05}").into())),
                field(3, OwnedValue::Utf8("fixture:source".into())),
            ])
        })
        .collect();
    for (table, tag) in [(3, 8), (4, 7)] {
        replace_field(&mut object, table, tag, evidence_keys(&["key-65536"]));
    }
    let bytes = raw_bytes(&object);
    let view = preflight_object_values(
        &bytes,
        PortableObjectKind::CanonicalArtifact,
        FormatLimits::HARD,
    )
    .unwrap()
    .registry_view();
    assert_eq!(view.section(3).unwrap().table(1).unwrap().chunk_count(), 2);
    let mut scratch = Scratch::new(u64::MAX);
    let index = ArtifactIndex::build(view, MISMATCH, &mut scratch).unwrap();
    validate_policy_references(&index, &mut scratch, MISMATCH).unwrap();
    object.sections[3].tables[1].rows[65_536].fields[1].value =
        OwnedValue::Utf8("key-65535".into());
    assert!(matches!(
        check(&object, u64::MAX),
        Err(PortableEmissionError::Format(
            FormatError::BindingMismatch { .. }
        ))
    ));
}

#[test]
fn artifact_diff_rejects_dangling_policy_in_both_roots() {
    let good = fixture();
    let mut bad = good.clone();
    replace_field(&mut bad, 3, 3, OwnedValue::U32(u32::MAX));
    for (base_object, target_object, expected) in [
        (&bad, &good, MISMATCH),
        (&good, &bad, PortableEmissionError::InternalBindingMismatch),
    ] {
        let base_bytes = raw_bytes(base_object);
        let base = preflight_object_values(
            &base_bytes,
            PortableObjectKind::CanonicalArtifact,
            FormatLimits::HARD,
        )
        .unwrap();
        let target = close_object(raw_bytes(target_object));
        assert_eq!(
            build_artifact_lfsd(
                base,
                NetworkRevisionId::from_digest(Sha256Digest::ZERO),
                &target,
                FormatLimits::HARD,
                u64::MAX
            ),
            Err(expected)
        );
    }
}

#[test]
fn policy_reference_fixture_matches_frozen_bytes() {
    let mut object = fixture();
    let revision = network_revision(&raw_bytes(&object), FormatLimits::HARD).unwrap();
    set_lfca_network_revision(&mut object, revision).unwrap();
    let bytes = raw_bytes(&object);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/portable/lfca-policy-references");
    if std::env::var_os("DUMP_PORTABLE").is_some() {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("expected.lfca"), &bytes).unwrap();
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
    let expected =
        include_bytes!("../../../../tests/fixtures/portable/lfca-policy-references/expected.lfca");
    assert_eq!(bytes.as_ref(), expected);
    assert_eq!(bytes.len(), 32_066);
    assert_eq!(
        object_key(sha256(&bytes)).as_ref(),
        "sha256/5cb385b1f87282c880ca7979200d1be602229b1c71c39b57123a777d9177c959"
    );
    laneflow_format::check_canonical_network_input(bytes.as_ref(), FormatLimits::HARD).unwrap();
    check(&object, u64::MAX).unwrap();
}
