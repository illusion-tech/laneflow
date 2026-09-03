use super::*;

fn many_policy_fixture(count: u32, all_member_kinds: bool) -> Fixture {
    let mut fixture = empty_policy_fixture();
    fixture
        .output
        .set_test_compile_limits(crate::CompileLimits::single_network_1m_v2());
    let mut unit = full_spatial_portable_fixture_unit();
    unit.limits = crate::CompileLimits::single_network_1m_v2();
    let module = unit
        .modules
        .iter()
        .position(|m| m.descriptor().source_language() == crate::SourceLanguage::SyntheticDsl)
        .unwrap();
    let namespace = unit.modules[module].descriptor().authoring_namespace_id();
    let document = unit
        .source_document_descriptors()
        .find(|d| d.authoring_namespace_id() == namespace)
        .unwrap()
        .source_document_key();
    let template = own_object(
        include_bytes!("../../../../tests/fixtures/portable/lfca-policy-references/expected.lfca"),
        PortableObjectKind::CanonicalArtifact,
    );
    let identity = template.sections[1].tables[0]
        .rows
        .iter()
        .find(|r| r.fields[0].value == OwnedValue::U16(24))
        .unwrap();
    let mut artifact = own_object(
        fixture.artifact.bytes(),
        PortableObjectKind::CanonicalArtifact,
    );
    let mut identities = artifact.sections[1].tables[0].rows.to_vec();
    let mut policies = Vec::new();
    let mut members: [Vec<OwnedRow>; 4] = core::array::from_fn(|_| Vec::new());
    let mut source_rows = Vec::new();
    for ordinal in 0..count {
        let key = format!("policy-{ordinal:06}");
        let id = crate::derive_canonical_stable_id_v1(
            EntityKind::RightOfWayPolicySet,
            namespace,
            &key,
            &unit.limits,
        )
        .unwrap()
        .into_bytes();
        let mut identity = identity.clone();
        *value(&mut identity, 2) = OwnedValue::U32(ordinal);
        *value(&mut identity, 3) = OwnedValue::StableId128(id);
        let OwnedValue::RecordVector(fields) = value(&mut identity, 4) else {
            panic!()
        };
        *value(&mut fields[0], 2) = OwnedValue::Bytes(namespace.as_bytes().into());
        *value(&mut fields[1], 2) = OwnedValue::Bytes(key.as_bytes().into());
        identities.push(identity);
        let mut policy = template.sections[2].tables[23].rows[0].clone();
        *value(&mut policy, 1) = OwnedValue::U32(ordinal);
        *value(&mut policy, 2) = OwnedValue::StableId128(id);
        policies.push(policy);
        source_rows.push((
            PolicySourceTarget::Declaration {
                id: StableId::from_untyped(StableId128::from_bytes(id)),
                ordinal: Ordinal::from_raw(ordinal),
            },
            SourceLocation::from(SourceSpan::point(document.into(), ordinal * 5 + 10, 1)),
        ));
        for (kind, rows) in members
            .iter_mut()
            .take(if all_member_kinds { 4 } else { 1 })
            .enumerate()
        {
            let mut member = template.sections[3].tables[kind + 1].rows[0].clone();
            *value(&mut member, 1) = OwnedValue::U32(ordinal);
            let OwnedValue::Utf8(key) = &member.fields[1].value else {
                panic!()
            };
            source_rows.push((
                PolicySourceTarget::Member {
                    owner: StableId::from_untyped(StableId128::from_bytes(id)),
                    kind: PolicyLocalMemberKind::from_code(kind as u8).unwrap(),
                    key: key.clone(),
                },
                SourceLocation::from(SourceSpan::point(
                    document.into(),
                    ordinal * 5 + 11 + kind as u32,
                    1,
                )),
            ));
            rows.push(member);
        }
    }
    artifact.sections[1].tables[0].rows = identities.into();
    artifact.sections[2].tables[23].rows = policies.into();
    for (kind, rows) in members.into_iter().enumerate() {
        artifact.sections[3].tables[kind + 1].rows = rows.into();
    }
    // 来源物理顺序与规范策略/成员顺序相反，每个策略拥有自己的成员。
    let inputs: Vec<_> = source_rows
        .iter()
        .rev()
        .map(|(target, primary)| PolicySourceInput {
            target: target.clone(),
            owner_module: module as u32,
            primary,
            contributing: &[],
        })
        .collect();
    fixture
        .output
        .test_source_map_mut()
        .set_test_policy_sources(&unit, &inputs)
        .unwrap();
    let revision = network_revision(&bytes(&artifact), FormatLimits::HARD).unwrap();
    set_lfca_network_revision(&mut artifact, revision).unwrap();
    fixture.artifact = close_object(bytes(&artifact));
    fixture.map = build_lfsm(
        &fixture.output,
        &PortableEmissionProvenance::try_new("policy-source-review-regression").unwrap(),
        source_collection_digest(&fixture.output).unwrap(),
        revision,
        &fixture.artifact,
    )
    .unwrap();
    fixture
}

fn check_changed_root(
    fixture: &Fixture,
    mut artifact: OwnedObject,
) -> Result<(), PortableEmissionError> {
    let revision = network_revision(&bytes(&artifact), FormatLimits::HARD).unwrap();
    set_lfca_network_revision(&mut artifact, revision).unwrap();
    let artifact = close_object(bytes(&artifact));
    let mut map = fixture.map.clone();
    let binding = &mut map.sections[0].tables[0].rows[0];
    *value(binding, 2) = OwnedValue::Sha256(revision.into_digest().into_bytes());
    *value(binding, 4) = OwnedValue::Sha256(artifact.digest().into_bytes());
    *value(binding, 5) = OwnedValue::U64(artifact.byte_length().get());
    check_portable_policy_sources(
        artifact.bytes(),
        fixture.output.source_map_input(),
        &bytes(&map),
        FormatLimits::HARD,
        fixture.output.compile_limits(),
    )
}

#[test]
fn many_policy_owners_and_identities_close_across_distinct_chunk_boundaries() {
    let fixture = many_policy_fixture(65_537, false);
    let root = preflight_object_values(
        fixture.artifact.bytes(),
        PortableObjectKind::CanonicalArtifact,
        FormatLimits::HARD,
    )
    .unwrap()
    .registry_view();
    for (section, table) in [(1, 0), (2, 23), (3, 1)] {
        assert!(
            root.section(section)
                .unwrap()
                .table(table)
                .unwrap()
                .chunk_count()
                >= 2
        );
    }
    // Identity 还包含前序实体，分块边界与策略实体表不对齐。
    assert!(root.section(1).unwrap().table(0).unwrap().row_count() > 65_537);
    let started = std::time::Instant::now();
    fixture.check(&fixture.map).unwrap();
    eprintln!(
        "65,537 policies with one member each: {:?}",
        started.elapsed()
    );
}

#[test]
fn policy_owner_and_identity_mismatches_fail_after_exact_rebinding() {
    let fixture = many_policy_fixture(4, true);
    fixture.check(&fixture.map).unwrap();
    let original = own_object(
        fixture.artifact.bytes(),
        PortableObjectKind::CanonicalArtifact,
    );
    for table in 1..=4 {
        let mut artifact = original.clone();
        *value(&mut artifact.sections[3].tables[table].rows[3], 1) = OwnedValue::U32(4);
        assert_eq!(
            check_changed_root(&fixture, artifact),
            Err(PortableEmissionError::PolicySourceMismatch)
        );
    }
    for mutation in 1..7 {
        let mut artifact = original.clone();
        let identities = &mut artifact.sections[1].tables[0].rows;
        let first = identities
            .iter()
            .position(|r| r.fields[0].value == OwnedValue::U16(24))
            .unwrap();
        match mutation {
            1 => *value(&mut identities[first], 2) = OwnedValue::U32(1),
            2 => {
                let foreign = value(&mut identities[first + 1], 3).clone();
                *value(&mut identities[first], 3) = foreign;
            }
            3 => identities.swap(first, first + 1),
            4 => *identities = identities[..identities.len() - 1].into(),
            5 => {
                let mut extra = identities.to_vec();
                extra.push(extra.last().unwrap().clone());
                *identities = extra.into();
            }
            6 => identities[first] = identities[first - 1].clone(),
            _ => unreachable!(),
        }
        assert!(
            check_changed_root(&fixture, artifact).is_err(),
            "mutation {mutation}"
        );
    }
    fixture.check(&fixture.map).unwrap();
}
