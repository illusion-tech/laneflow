use super::*;
use laneflow_static_network::{
    BuildError, BuildStructure, PolicyBuildViolation as V, SharedNetworkBuildLimits,
    SharedNetworkBuildOptions, SpatialBuildOption, build_shared_network_revision,
};

const VALID: &[u8] =
    include_bytes!("../../../../tests/fixtures/portable/lfca-world-policies/expected.lfca");
const LIMITS: SharedNetworkBuildLimits =
    SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024);

#[test]
fn shared_root_retains_empty_optional_evidence_description() {
    let mut object = own_object(VALID, PortableObjectKind::CanonicalArtifact);
    let row = &mut object.sections[3].tables[1].rows[0];
    let mut fields = row.fields.to_vec();
    fields.push(field(4, OwnedValue::Utf8("".into())));
    row.fields = fields.into();
    build(&artifact(object), SpatialBuildOption::Omit, LIMITS).unwrap();
}
fn artifact(mut object: OwnedObject) -> Box<[u8]> {
    let revision = network_revision(&bytes(&object), FormatLimits::HARD).unwrap();
    set_lfca_network_revision(&mut object, revision).unwrap();
    bytes(&object)
}
fn build(
    bytes: &[u8],
    spatial: SpatialBuildOption,
    limits: SharedNetworkBuildLimits,
) -> Result<Arc<laneflow_static_network::SharedNetworkRevision>, BuildError> {
    let checked =
        laneflow_format::check_canonical_network_input(bytes, FormatLimits::HARD).unwrap();
    build_shared_network_revision(checked, SharedNetworkBuildOptions::new(spatial, limits))
}
fn rejects(mutate: impl FnOnce(&mut OwnedObject), expected: V) {
    rejects_input(VALID, mutate, expected);
}
fn rejects_input(input: &[u8], mutate: impl FnOnce(&mut OwnedObject), expected: V) {
    let mut object = own_object(input, PortableObjectKind::CanonicalArtifact);
    mutate(&mut object);
    let artifact = artifact(object);
    for spatial in [
        SpatialBuildOption::Omit,
        SpatialBuildOption::RetainAvailable,
    ] {
        match build(&artifact, spatial, LIMITS) {
            Err(BuildError::Policy { violation, .. }) => assert_eq!(violation, expected),
            Err(error) => panic!("unexpected closure error: {error:?}"),
            Ok(_) => panic!("shared root must independently reject {expected:?}"),
        }
    }
}

#[test]
fn shared_root_independently_closes_signal_direction_lamp_and_protected_rules() {
    const SIGNAL: &[u8] =
        include_bytes!("../../../../tests/fixtures/portable/lfca-world-policies/signal.lfca");
    rejects(
        |o| {
            *value(&mut o.sections[3].tables[4].rows[0], 5) = OwnedValue::U8(1);
        },
        V::SignalBinding,
    );
    rejects_input(
        SIGNAL,
        |o| {
            for row in &mut o.sections[2].tables[5].rows {
                *value(row, 7) = OwnedValue::U8(0);
            }
        },
        V::RightTurnRequired,
    );
    // 同一实体在另一份策略中的普通圆灯声明仍然有效，不被更早规则覆盖。
    rejects_input(
        SIGNAL,
        |o| {
            *value(&mut o.sections[3].tables[4].rows[0], 5) = OwnedValue::U8(4);
        },
        V::LampTypeConflict,
    );
    rejects_input(
        SIGNAL,
        |o| {
            for row in &mut o.sections[3].tables[4].rows {
                *value(row, 5) = OwnedValue::U8(1);
                *value(row, 6) = OwnedValue::U8(0);
            }
        },
        V::ProtectedConflict,
    );
}

#[test]
fn shared_root_rejects_ambiguous_nearest_rules() {
    rejects(
        |o| {
            let mut rows = o.sections[3].tables[4].rows.to_vec();
            let mut duplicate = rows[0].clone();
            *value(&mut duplicate, 2) = OwnedValue::Utf8("z-shadow".into());
            rows.insert(2, duplicate);
            o.sections[3].tables[4].rows = rows.into();
        },
        V::AmbiguousRule,
    );
}

#[test]
fn shared_root_never_completes_a_policy_from_another_policy() {
    rejects(
        |o| {
            o.sections[3].tables[4].rows = o.sections[3].tables[4].rows[1..].to_vec().into();
        },
        V::MissingRule,
    );
    rejects(
        |o| {
            o.sections[3].tables[3].rows = o.sections[3].tables[3].rows[1..].to_vec().into();
        },
        V::MissingRule,
    );
}

#[test]
fn shared_root_checks_local_references_evidence_and_target_priority() {
    rejects(
        |o| {
            *value(&mut o.sections[3].tables[4].rows[0], 7) =
                OwnedValue::RecordVector(Box::new([]));
        },
        V::Evidence,
    );
    rejects(
        |o| {
            *value(
                o.sections[3].tables[3]
                    .rows
                    .iter_mut()
                    .find(|r| r.fields.iter().any(|f| f.tag == 7))
                    .unwrap(),
                7,
            ) = OwnedValue::Utf8("unknown".into());
        },
        V::Reference,
    );
    rejects(
        |o| {
            let row = o.sections[3].tables[3].rows.iter_mut().find(|r| matches!(&r.fields.iter().find(|f| f.tag == 6).unwrap().value, OwnedValue::OrdinalVectorU32(v) if !v.is_empty())).unwrap();
            *value(row, 5) = OwnedValue::I32(i32::MAX);
        },
        V::YieldPriority,
    );
    rejects(
        |o| {
            let row = o.sections[3].tables[3].rows.iter_mut().find(|r| matches!(&r.fields.iter().find(|f| f.tag == 6).unwrap().value, OwnedValue::OrdinalVectorU32(v) if !v.is_empty())).unwrap();
            let OwnedValue::U32(owner) = *value(row, 3) else {
                panic!()
            };
            *value(row, 6) = OwnedValue::OrdinalVectorU32(vec![owner].into());
        },
        V::SelfYield,
    );
}

#[test]
fn shared_root_accepts_all_explicit_directions_without_inferring_a_right_turn() {
    for direction in 0..=3 {
        let mut o = own_object(VALID, PortableObjectKind::CanonicalArtifact);
        for row in &mut o.sections[2].tables[5].rows {
            let mut fields = row.fields.to_vec();
            fields.push(field(7, OwnedValue::U8(direction)));
            row.fields = fields.into();
        }
        let bytes = artifact(o);
        build(&bytes, SpatialBuildOption::Omit, LIMITS).unwrap();
    }
}

#[test]
fn shared_root_policy_budgets_fail_atomically_and_exact_retained_limit_retries() {
    let root = build(VALID, SpatialBuildOption::Omit, LIMITS).unwrap();
    let retained = root.retained_logical_bytes();
    assert!(matches!(
        build(
            VALID,
            SpatialBuildOption::Omit,
            SharedNetworkBuildLimits::new(retained - 1, LIMITS.max_scratch_bytes())
        ),
        Err(BuildError::BudgetExceeded {
            structure: BuildStructure::RetainedOutput,
            ..
        })
    ));
    assert_eq!(
        build(
            VALID,
            SpatialBuildOption::Omit,
            SharedNetworkBuildLimits::new(retained, LIMITS.max_scratch_bytes())
        )
        .unwrap()
        .retained_logical_bytes(),
        retained
    );
    assert!(matches!(
        build(
            VALID,
            SpatialBuildOption::Omit,
            LIMITS.with_max_policy_work(1)
        ),
        Err(BuildError::BudgetExceeded {
            structure: BuildStructure::PolicyWork,
            ..
        })
    ));
    build(VALID, SpatialBuildOption::Omit, LIMITS).unwrap();
}

#[test]
fn shared_root_independently_rejects_access_policy_regulation_mismatch() {
    const FULL: &[u8] =
        include_bytes!("../../../../tests/fixtures/portable/lfca-world-policies/full-spatial.lfca");
    for tag in [3, 4] {
        rejects_input(
            FULL,
            |object| {
                *value(&mut object.sections[2].tables[23].rows[0], tag) =
                    OwnedValue::Utf8("different".into());
            },
            V::RegulationMismatch,
        );
    }
}
