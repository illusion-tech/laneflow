use super::*;
use crate::compiler::portable_fixture_tests::{
    full_spatial_portable_fixture_output, full_spatial_portable_fixture_unit,
};
use crate::source_map::policy::PolicySourceInput;
use crate::{PolicySourceTarget, SourceLocation, SourceSpan};
use laneflow_static_contract::{PolicyLocalMemberKind, StableId128};
use std::sync::Arc;

fn own(row_ref: RegistryCheckedRowView<'_>) -> OwnedRow {
    row(row_ref.fields().map(|f| {
        field(
            f.tag(),
            match f.value().unwrap() {
                RegistryCheckedFieldValue::U8(v) => OwnedValue::U8(v),
                RegistryCheckedFieldValue::U16(v) => OwnedValue::U16(v),
                RegistryCheckedFieldValue::U32(v) => OwnedValue::U32(v),
                RegistryCheckedFieldValue::U64(v) => OwnedValue::U64(v),
                RegistryCheckedFieldValue::I32(v) => OwnedValue::I32(v),
                RegistryCheckedFieldValue::F32(v) => OwnedValue::F32(v),
                RegistryCheckedFieldValue::F64(v) => OwnedValue::F64(v),
                RegistryCheckedFieldValue::Utf8(v) => OwnedValue::Utf8(v.into()),
                RegistryCheckedFieldValue::Bytes(v) => OwnedValue::Bytes(v.into()),
                RegistryCheckedFieldValue::StableId128(v) => {
                    OwnedValue::StableId128(v.into_bytes())
                }
                RegistryCheckedFieldValue::Sha256(v) => OwnedValue::Sha256(v.into_bytes()),
                RegistryCheckedFieldValue::OrdinalVectorU32(v) => {
                    OwnedValue::OrdinalVectorU32((0..v.len()).map(|i| v.get(i).unwrap()).collect())
                }
                RegistryCheckedFieldValue::RecordVector(v) => {
                    OwnedValue::RecordVector(v.rows().map(own).collect())
                }
            },
        )
    }))
}
fn own_object(bytes: &[u8], kind: PortableObjectKind) -> OwnedObject {
    let view = preflight_object_values(bytes, kind, FormatLimits::HARD)
        .unwrap()
        .registry_view();
    OwnedObject {
        kind,
        sections: view
            .sections()
            .map(|s| {
                section(
                    s.kind(),
                    s.tables().map(|t| table(t.kind(), t.rows().map(own))),
                )
            })
            .collect(),
    }
}
fn bytes(object: &OwnedObject) -> Box<[u8]> {
    encode_owned_object(object, FormatLimits::HARD, None).unwrap()
}
fn value(row: &mut OwnedRow, tag: u16) -> &mut OwnedValue {
    &mut row.fields.iter_mut().find(|f| f.tag == tag).unwrap().value
}
fn stable(row: &OwnedRow) -> [u8; 16] {
    let OwnedValue::StableId128(v) = row.fields[1].value else {
        panic!()
    };
    v
}

struct Fixture {
    output: CompilationOutput,
    artifact: PortableObjectCandidate,
    map: OwnedObject,
}

fn empty_policy_fixture() -> Fixture {
    let output = full_spatial_portable_fixture_output();
    let provenance =
        PortableEmissionProvenance::try_new("policy-source-review-regression").unwrap();
    let candidate = emit_portable_candidate(
        &output,
        &provenance,
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .unwrap();
    Fixture {
        output,
        artifact: candidate.canonical_artifact().clone(),
        map: own_object(
            candidate.source_map().bytes(),
            PortableObjectKind::SourceMap,
        ),
    }
}

#[test]
fn module_primary_must_equal_its_source_even_when_the_pool_is_unchanged() {
    for fixture in [empty_policy_fixture(), fixture(true, None, 2)] {
        fixture.check(&fixture.map).unwrap();
        let mut swapped = fixture.map.clone();
        let first = value(&mut swapped.sections[1].tables[0].rows[0], 13).clone();
        let second = value(&mut swapped.sections[1].tables[0].rows[1], 13).clone();
        assert_ne!(first, second);
        *value(&mut swapped.sections[1].tables[0].rows[0], 13) = second;
        *value(&mut swapped.sections[1].tables[0].rows[1], 13) = first;
        assert_eq!(
            fixture.check(&swapped),
            Err(PortableEmissionError::PolicySourceMismatch)
        );
    }
    let fixture = empty_policy_fixture();
    let mut wrong_span = fixture.map.clone();
    let OwnedValue::U32(primary) = *value(&mut wrong_span.sections[1].tables[0].rows[0], 13) else {
        panic!()
    };
    let alternative = wrong_span.sections[1].tables[2]
        .rows
        .iter_mut()
        .find_map(|row| {
            let ordinal = value(row, 1).clone();
            if *value(row, 3) == OwnedValue::U32(0) && ordinal != OwnedValue::U32(primary) {
                Some(ordinal)
            } else {
                None
            }
        })
        .unwrap();
    *value(&mut wrong_span.sections[1].tables[0].rows[0], 13) = alternative;
    // 另一个既有关系继续引用原位置，使池完整性无法单独发现模块位置被换掉。
    let OwnedValue::OrdinalVectorU32(contributing) =
        value(&mut wrong_span.sections[3].tables[0].rows[0], 6)
    else {
        panic!()
    };
    let mut ordinals = contributing.to_vec();
    ordinals.push(primary);
    ordinals.sort_unstable();
    ordinals.dedup();
    *contributing = ordinals.into();
    assert_eq!(
        fixture.check(&wrong_span),
        Err(PortableEmissionError::PolicySourceMismatch)
    );
}

#[test]
fn empty_policy_fast_path_rejects_each_orphan_member_table() {
    let fixture = empty_policy_fixture();
    let members = own_object(
        include_bytes!("../../../tests/fixtures/portable/lfca-policy-references/expected.lfca"),
        PortableObjectKind::CanonicalArtifact,
    );
    fixture.check(&fixture.map).unwrap();
    for table in 1..=4 {
        let mut artifact = own_object(
            fixture.artifact.bytes(),
            PortableObjectKind::CanonicalArtifact,
        );
        assert!(artifact.sections[2].tables[23].rows.is_empty());
        artifact.sections[3].tables[table].rows =
            [members.sections[3].tables[table].rows[0].clone()].into();
        let revision = network_revision(&bytes(&artifact), FormatLimits::HARD).unwrap();
        set_lfca_network_revision(&mut artifact, revision).unwrap();
        let artifact = close_object(bytes(&artifact));
        let mut map = fixture.map.clone();
        let binding = &mut map.sections[0].tables[0].rows[0];
        *value(binding, 2) = OwnedValue::Sha256(revision.into_digest().into_bytes());
        *value(binding, 4) = OwnedValue::Sha256(artifact.digest().into_bytes());
        *value(binding, 5) = OwnedValue::U64(artifact.byte_length().get());
        assert_eq!(
            check_portable_policy_sources(
                artifact.bytes(),
                fixture.output.source_map_input(),
                &bytes(&map),
                FormatLimits::HARD,
                &crate::CompileLimits::single_network_1m_v2()
            ),
            Err(PortableEmissionError::PolicySourceMismatch),
            "member table {table}"
        );
    }
    fixture.check(&fixture.map).unwrap();
}
impl Fixture {
    fn check(&self, map: &OwnedObject) -> Result<(), PortableEmissionError> {
        check_portable_policy_sources(
            self.artifact.bytes(),
            self.output.source_map_input(),
            &bytes(map),
            FormatLimits::HARD,
            &crate::CompileLimits::single_network_1m_v2(),
        )
    }
}

fn fixture(road: bool, canvas: Option<&str>, evidence_count: usize) -> Fixture {
    let mut output = full_spatial_portable_fixture_output();
    let mut unit = full_spatial_portable_fixture_unit();
    if evidence_count > 10_000 {
        unit.limits = crate::CompileLimits::single_network_1m_v2();
        output.set_test_compile_limits(crate::CompileLimits::single_network_1m_v2());
    }
    let module_index = unit
        .modules
        .iter()
        .position(|m| {
            (m.descriptor().source_language() == crate::SourceLanguage::RoadEditingSource) == road
        })
        .unwrap();
    let namespace = unit.modules[module_index]
        .descriptor()
        .authoring_namespace_id();
    let document = unit
        .source_document_descriptors()
        .find(|d| d.authoring_namespace_id() == namespace)
        .unwrap()
        .source_document_key();
    let provenance =
        PortableEmissionProvenance::try_new("laneflow-fixture-284-policy-source-v1").unwrap();
    let collection = source_collection_digest(&output).unwrap();
    let mut artifact = own_object(
        include_bytes!("../../../tests/fixtures/portable/lfca-policy-references/expected.lfca"),
        PortableObjectKind::CanonicalArtifact,
    );
    let fresh = lfca::build_lfca(
        &output,
        &provenance,
        collection,
        NetworkRevisionId::from_digest(Sha256Digest::ZERO),
    )
    .unwrap();
    artifact.sections[6] = fresh.sections[6].clone();
    let mut policy_ids = Vec::new();
    for (i, key) in ["a", "b"].iter().enumerate() {
        let id = crate::derive_canonical_stable_id_v1(
            EntityKind::RightOfWayPolicySet,
            namespace,
            key,
            &crate::CompileLimits::single_network_1m_v2(),
        )
        .unwrap()
        .into_bytes();
        policy_ids.push(id);
        let identity = artifact.sections[1].tables[0]
            .rows
            .iter_mut()
            .filter(|r| r.fields[0].value == OwnedValue::U16(24))
            .nth(i)
            .unwrap();
        *value(identity, 3) = OwnedValue::StableId128(id);
        let OwnedValue::RecordVector(fields) = value(identity, 4) else {
            panic!()
        };
        *value(&mut fields[0], 2) = OwnedValue::Bytes(namespace.as_bytes().into());
        *value(&mut artifact.sections[2].tables[23].rows[i], 2) = OwnedValue::StableId128(id);
    }
    let evidence = artifact.sections[3].tables[1].rows[0].clone();
    artifact.sections[3].tables[1].rows = (0..evidence_count)
        .map(|i| {
            let mut row = evidence.clone();
            *value(&mut row, 2) = OwnedValue::Utf8(if i == 0 {
                "a".into()
            } else {
                format!("k{i:06}").into()
            });
            row
        })
        .collect();
    // 一个 Movement 的方向只增加其字段贡献来源。
    let movement_index = artifact.sections[1].tables[0]
        .rows
        .iter()
        .filter(|r| r.fields[0].value == OwnedValue::U16(6))
        .find_map(|r| {
            let OwnedValue::RecordVector(ref fields) = r.fields[3].value else {
                return None;
            };
            if fields[0].fields[1].value != OwnedValue::Bytes(namespace.as_bytes().into()) {
                return None;
            }
            let OwnedValue::U32(i) = r.fields[1].value else {
                return None;
            };
            Some(i as usize)
        })
        .unwrap();
    let movement = &mut artifact.sections[2].tables[5].rows[movement_index];
    let mut fields = movement.fields.to_vec();
    fields.push(field(7, OwnedValue::U8(0)));
    movement.fields = fields.into();
    let movement_id = stable(movement);
    let mut inputs_owned: Vec<(PolicySourceTarget, SourceLocation, Vec<SourceLocation>)> =
        Vec::new();
    for (ordinal, id) in policy_ids.iter().enumerate() {
        let primary = make_location(
            (namespace, document),
            if ordinal == 0 { "a" } else { "b" },
            road,
            canvas,
            None,
            None,
            10 + ordinal as u32,
        );
        let paths = [
            vec![(40, 0)],
            vec![(40, 1), (30, 0)],
            vec![(40, 1), (30, 1)],
            vec![(40, 1), (30, 2)],
        ];
        let contributing = paths
            .iter()
            .enumerate()
            .map(|(i, path)| {
                make_location(
                    (namespace, document),
                    if ordinal == 0 { "a" } else { "b" },
                    road,
                    canvas,
                    None,
                    Some(path),
                    20 + i as u32,
                )
            })
            .collect();
        inputs_owned.push((
            PolicySourceTarget::Declaration {
                id: StableId::from_untyped(StableId128::from_bytes(*id)),
                ordinal: Ordinal::from_raw(ordinal as u32),
            },
            primary,
            contributing,
        ));
    }
    for kind in 0..4_u8 {
        for (i, member) in artifact.sections[3].tables[usize::from(kind) + 1]
            .rows
            .iter()
            .enumerate()
        {
            let OwnedValue::Utf8(ref key) = member.fields[1].value else {
                panic!()
            };
            let primary_path = [(40, u16::from(kind) + 2)];
            let primary = make_location(
                (namespace, document),
                "a",
                road,
                canvas,
                Some((kind, i as u32)),
                Some(&primary_path),
                100 + u32::from(kind) * 10 + i as u32,
            );
            let contributing = member
                .fields
                .iter()
                .filter(|f| f.tag >= 2)
                .map(|f| {
                    make_location(
                        (namespace, document),
                        "a",
                        road,
                        canvas,
                        Some((kind, i as u32)),
                        Some(&[(40, u16::from(kind) + 2), (u16::from(kind) + 41, f.tag - 2)]),
                        500 + u32::from(kind) * 10 + u32::from(f.tag),
                    )
                })
                .collect();
            inputs_owned.push((
                PolicySourceTarget::Member {
                    owner: StableId::from_untyped(StableId128::from_bytes(policy_ids[0])),
                    kind: PolicyLocalMemberKind::from_code(kind).unwrap(),
                    key: key.clone(),
                },
                primary,
                contributing,
            ));
        }
    }
    let movement_source = output
        .source_map_input()
        .movement_sources()
        .find(|s| stable_id_bytes(s.stable_id()) == movement_id)
        .unwrap();
    let direction = if road {
        let l = movement_source.primary_source().road_editing().unwrap();
        let crate::RoadEditingSubject::Declaration { address } = l.subject() else {
            panic!()
        };
        make_movement_location(l, *address)
    } else {
        SourceSpan::point(document.into(), 800, 1).into()
    };
    inputs_owned.push((
        PolicySourceTarget::MovementDirection {
            id: StableId::from_untyped(StableId128::from_bytes(movement_id)),
        },
        direction,
        Vec::new(),
    ));
    inputs_owned.reverse(); // 物理来源顺序与规范 key 序刻意不同。
    let inputs: Vec<_> = inputs_owned
        .iter()
        .map(|(target, primary, contributing)| PolicySourceInput {
            target: target.clone(),
            owner_module: module_index as u32,
            primary,
            contributing,
        })
        .collect();
    output
        .test_source_map_mut()
        .set_test_policy_sources(&unit, &inputs)
        .unwrap();
    let revision = network_revision(&bytes(&artifact), FormatLimits::HARD).unwrap();
    set_lfca_network_revision(&mut artifact, revision).unwrap();
    let artifact = close_object(bytes(&artifact));
    let map = build_lfsm(&output, &provenance, collection, revision, &artifact).unwrap();
    Fixture {
        output,
        artifact,
        map,
    }
}

fn table_kind(code: u16) -> crate::RoadEditingTableKind {
    use crate::RoadEditingTableKind::*;
    match code {
        14 => Movement,
        30 => AccessRegulation,
        40 => RightOfWayPolicySet,
        41 => PolicyEvidence,
        42 => PolicyGapProfile,
        43 => PolicyStreamRule,
        44 => PolicyGateRule,
        _ => panic!(),
    }
}
fn make_location(
    source: (&str, &str),
    owner_key: &str,
    road: bool,
    canvas: Option<&str>,
    member: Option<(u8, u32)>,
    path: Option<&[(u16, u16)]>,
    line: u32,
) -> SourceLocation {
    let (namespace, document) = source;
    if !road {
        return SourceSpan::point(document.into(), line, 1).into();
    }
    let strings: Box<[Arc<str>]> = [Arc::from(namespace), Arc::from(owner_key)]
        .into_iter()
        .collect();
    let paths = path
        .map(|p| {
            crate::RoadEditingPropertyPath::new(
                p.iter()
                    .map(|(c, f)| crate::RoadEditingPropertyStep::TableField {
                        table: table_kind(*c),
                        field_id: *f,
                    })
                    .collect(),
            )
        })
        .into_iter()
        .collect();
    let context = Arc::new(crate::RoadEditingLocationContext::new(
        strings,
        paths,
        canvas.map(Arc::from).into_iter().collect(),
    ));
    let address = crate::RoadEditingSourceAddress::new(
        context.string_ordinal(0),
        crate::RoadEditingAddressKind::Declaration(EntityKind::RightOfWayPolicySet),
        [],
        context.string_ordinal(1),
    );
    let subject = if let Some((kind, index)) = member {
        let relation = [
            crate::RoadEditingRelationKind::PolicyEvidence,
            crate::RoadEditingRelationKind::PolicyGapProfile,
            crate::RoadEditingRelationKind::PolicyStreamRule,
            crate::RoadEditingRelationKind::PolicyGateRule,
        ][kind as usize];
        crate::RoadEditingSubject::OwnerLocal {
            owner: crate::RoadEditingOwner::Address(address),
            relation,
            occurrence: crate::RoadEditingRelationOccurrence::CanonicalSetOrdinal(index),
        }
    } else {
        crate::RoadEditingSubject::Declaration { address }
    };
    let property = path.map(|_| context.property_path_ordinal(0));
    let canvas = canvas.map(|_| context.canvas_selection_ordinal(0));
    SourceLocation::RoadEditing(crate::RoadEditingSourceLocation::new(
        context,
        crate::RoadEditingDocumentIdentity::verified(namespace.into(), document.into()),
        subject,
        property,
        canvas,
        None,
    ))
}

fn make_movement_location(
    location: &crate::RoadEditingSourceLocation,
    address: crate::RoadEditingSourceAddress,
) -> SourceLocation {
    let namespace = address.module_namespace(location.context());
    let key = address.local_key(location.context());
    let parents: Vec<_> = address.owner_local_keys(location.context()).collect();
    let mut strings: Vec<Arc<str>> = vec![namespace.into(), key.into()];
    strings.extend(parents.iter().map(|s| Arc::from(*s)));
    let path = crate::RoadEditingPropertyPath::new(
        [crate::RoadEditingPropertyStep::TableField {
            table: crate::RoadEditingTableKind::Movement,
            field_id: 5,
        }]
        .into(),
    );
    let context = Arc::new(crate::RoadEditingLocationContext::new(
        strings.into(),
        [path].into(),
        location
            .canvas_selection()
            .map(Arc::from)
            .into_iter()
            .collect(),
    ));
    let address = crate::RoadEditingSourceAddress::new(
        context.string_ordinal(0),
        crate::RoadEditingAddressKind::Declaration(EntityKind::Movement),
        (0..parents.len()).map(|i| context.string_ordinal(i + 2)),
        context.string_ordinal(1),
    );
    let property = context.property_path_ordinal(0);
    let canvas = location
        .canvas_selection()
        .map(|_| context.canvas_selection_ordinal(0));
    SourceLocation::RoadEditing(crate::RoadEditingSourceLocation::new(
        context,
        location.document_identity().clone(),
        crate::RoadEditingSubject::Declaration { address },
        Some(property),
        canvas,
        None,
    ))
}

#[test]
fn policy_sources_close_text_road_members_direction_and_canvas() {
    for (road, canvas) in [
        (false, None),
        (true, None),
        (true, Some("")),
        (true, Some("policy-node")),
    ] {
        let fixture = fixture(road, canvas, 2);
        fixture.check(&fixture.map).unwrap();
        assert_eq!(
            fixture.map.sections[3].tables[0]
                .rows
                .iter()
                .filter(|r| r.fields[0].value == OwnedValue::U16(24))
                .count(),
            5
        );
    }
}

#[test]
fn policy_source_checker_rejects_missing_extra_and_forged_projections() {
    let fixture = fixture(true, Some("node"), 2);
    fixture.check(&fixture.map).unwrap();
    for mutation in 0..11 {
        let mut map = fixture.map.clone();
        let local = map.sections[3].tables[0]
            .rows
            .iter()
            .position(|r| r.fields[0].value == OwnedValue::U16(24))
            .unwrap();
        match mutation {
            0 => {
                let mut rows = map.sections[3].tables[0].rows.to_vec();
                rows.remove(local);
                map.sections[3].tables[0].rows = rows.into();
            }
            1 => {
                *value(&mut map.sections[3].tables[0].rows[local], 5) = OwnedValue::U32(0);
            }
            2 => {
                *value(&mut map.sections[3].tables[0].rows[local], 6) =
                    OwnedValue::OrdinalVectorU32(Box::new([]));
            }
            3 => {
                let other = map.sections[3].tables[0].rows[local + 1].fields[4]
                    .value
                    .clone();
                *value(&mut map.sections[3].tables[0].rows[local], 5) = other;
            }
            4 => {
                *value(&mut map.sections[3].tables[0].rows[local], 2) =
                    OwnedValue::StableId128([99; 16]);
            }
            5 => {
                *value(&mut map.sections[3].tables[0].rows[local], 4) = OwnedValue::U32(99);
            }
            6 => {
                let row = map.sections[2].tables[0]
                    .rows
                    .iter_mut()
                    .find(|r| r.fields[0].value == OwnedValue::U16(24))
                    .unwrap();
                *value(row, 5) = OwnedValue::OrdinalVectorU32(Box::new([]));
            }
            7 => {
                let mut rows = map.sections[3].tables[0].rows.to_vec();
                let mut extra = rows[local].clone();
                *value(&mut extra, 4) = OwnedValue::U32(99);
                rows.insert(local + 2, extra);
                map.sections[3].tables[0].rows = rows.into();
            }
            8 => {
                *value(&mut map.sections[3].tables[0].rows[local], 6) =
                    OwnedValue::OrdinalVectorU32([0].into());
            }
            9 => {
                let row = map.sections[2].tables[0]
                    .rows
                    .iter_mut()
                    .find(|r| r.fields[0].value == OwnedValue::U16(24))
                    .unwrap();
                *value(row, 4) = OwnedValue::U32(0);
            }
            10 => {
                *value(&mut map.sections[1].tables[0].rows[0], 8) =
                    OwnedValue::Utf8("forged-build".into());
            }
            _ => unreachable!(),
        }
        match encode_owned_object(&map, FormatLimits::HARD, None) {
            Err(_) => {}
            Ok(bytes) => assert!(
                check_portable_policy_sources(
                    fixture.artifact.bytes(),
                    fixture.output.source_map_input(),
                    &bytes,
                    FormatLimits::HARD,
                    &crate::CompileLimits::single_network_1m_v2()
                )
                .is_err(),
                "mutation {mutation}"
            ),
        }
    }
    fixture.check(&fixture.map).unwrap();
}

#[test]
fn policy_sources_keep_global_ordinals_across_real_chunks() {
    let fixture = fixture(false, None, 65_537);
    let bytes = bytes(&fixture.map);
    let view = preflight_object_values(&bytes, PortableObjectKind::SourceMap, FormatLimits::HARD)
        .unwrap()
        .registry_view();
    assert!(view.section(1).unwrap().table(2).unwrap().chunk_count() >= 2);
    assert!(view.section(3).unwrap().table(0).unwrap().chunk_count() >= 2);
    check_portable_policy_sources(
        fixture.artifact.bytes(),
        fixture.output.source_map_input(),
        &bytes,
        FormatLimits::HARD,
        &crate::CompileLimits::single_network_1m_v2(),
    )
    .unwrap();
    let mut wrong = fixture.map.clone();
    let row = wrong.sections[3].tables[0]
        .rows
        .iter_mut()
        .find(|r| {
            r.fields[0].value == OwnedValue::U16(24) && r.fields[3].value == OwnedValue::U32(65_536)
        })
        .unwrap();
    *value(row, 5) = OwnedValue::U32(0);
    assert!(fixture.check(&wrong).is_err());
}

#[test]
fn policy_source_limits_and_foreign_document_fail_without_partial_state() {
    let mut fixture = fixture(true, Some("node"), 1);
    let original = bytes(&fixture.map);
    let small = crate::CompileLimits::single_network_1m_v2()
        .with_test_admission_limit(CompileLimitDimension::StageScratchBytes, 1);
    assert!(matches!(
        check_portable_policy_sources(
            fixture.artifact.bytes(),
            fixture.output.source_map_input(),
            &original,
            FormatLimits::HARD,
            &small
        ),
        Err(PortableEmissionError::CompileLimitExceeded {
            dimension: CompileLimitDimension::StageScratchBytes,
            ..
        })
    ));
    let mut config = laneflow_format::FormatLimitConfig::HARD;
    config.max_rows_per_chunk = 1;
    let limits = FormatLimits::try_new(config).unwrap();
    assert!(
        check_portable_policy_sources(
            fixture.artifact.bytes(),
            fixture.output.source_map_input(),
            &original,
            limits,
            &crate::CompileLimits::single_network_1m_v2()
        )
        .is_err()
    );
    let before = fixture.output.source_map_input().policy_sources().len();
    let target = fixture
        .output
        .source_map_input()
        .policy_sources()
        .next()
        .unwrap()
        .target()
        .clone();
    let unit = full_spatial_portable_fixture_unit();
    let foreign: SourceLocation = SourceSpan::point("unregistered.document".into(), 1, 1).into();
    assert!(
        fixture
            .output
            .test_source_map_mut()
            .set_test_policy_sources(
                &unit,
                &[PolicySourceInput {
                    target: target.clone(),
                    owner_module: 0,
                    primary: &foreign,
                    contributing: &[]
                }]
            )
            .is_err()
    );
    assert_eq!(
        fixture.output.source_map_input().policy_sources().len(),
        before
    );
    let mut small_unit = full_spatial_portable_fixture_unit();
    let document = small_unit
        .source_document_descriptors()
        .next()
        .unwrap()
        .source_document_key()
        .to_owned();
    small_unit.limits = crate::CompileLimits::single_network_1m_v2()
        .with_test_admission_limit(CompileLimitDimension::RelationOccurrenceCount, 0);
    let real: SourceLocation = SourceSpan::point(document.into(), 1, 1).into();
    assert!(
        fixture
            .output
            .test_source_map_mut()
            .set_test_policy_sources(
                &small_unit,
                &[PolicySourceInput {
                    target,
                    owner_module: 0,
                    primary: &real,
                    contributing: &[]
                }]
            )
            .is_err()
    );
    assert_eq!(
        fixture.output.source_map_input().policy_sources().len(),
        before
    );
    fixture.check(&fixture.map).unwrap();
    let mut old = original.to_vec();
    old[4..6].copy_from_slice(&3_u16.to_le_bytes());
    assert!(
        check_portable_policy_sources(
            fixture.artifact.bytes(),
            fixture.output.source_map_input(),
            &old,
            FormatLimits::HARD,
            &crate::CompileLimits::single_network_1m_v2()
        )
        .is_err()
    );
}

#[test]
fn policy_source_fixture_matches_frozen_wire() {
    let fixture = fixture(true, Some(""), 2);
    fixture.check(&fixture.map).unwrap();
    let map = bytes(&fixture.map);
    if std::env::var_os("DUMP_PORTABLE").is_some() {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/portable/lfsm-policy-sources");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("expected.lfca"), fixture.artifact.bytes()).unwrap();
        std::fs::write(directory.join("expected.lfsm"), &map).unwrap();
    }
    assert_eq!(
        map.as_ref(),
        include_bytes!("../../../tests/fixtures/portable/lfsm-policy-sources/expected.lfsm")
    );
    assert_eq!(
        fixture.artifact.bytes(),
        include_bytes!("../../../tests/fixtures/portable/lfsm-policy-sources/expected.lfca")
    );
    assert_eq!(map.len(), 54_823);
    assert_eq!(
        object_key(sha256(&map)).as_ref(),
        "sha256/ba9290374c23e89ea719249d1f319f441112214bea0ef753c01670c16088a013"
    );
}
