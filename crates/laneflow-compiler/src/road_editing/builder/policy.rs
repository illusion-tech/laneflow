use super::*;

pub(super) fn charge(
    usage: &mut ModuleUsage,
    value: &RightOfWayPolicySetInput,
    namespace: &str,
    imports: &BTreeSet<Box<str>>,
    limits: &CompileLimits,
) -> Result<(), DiagnosticBundle> {
    usage.charge_table(7, 28);
    value.regulation.validate()?;
    usage.charge_table(3, 12);
    usage.typed_ast_record_count = usage.typed_ast_record_count.saturating_add(1);
    usage.charge_token(value.regulation.jurisdiction(), limits)?;
    usage.charge_token(value.regulation.version(), limits)?;
    if let Some(source) = value.regulation.source() {
        usage.charge_token(source, limits)?;
    }
    usage.relation_occurrence_count = usage
        .relation_occurrence_count
        .saturating_add(3 + u64::from(value.regulation.source().is_some()));
    usage.charge_canvas(value.canvas_selection(), limits)?;
    for count in [
        value.evidence.len(),
        value.gaps.len(),
        value.streams.len(),
        value.gates.len(),
    ] {
        usage.charge_vector(count, 4);
        usage.typed_ast_record_count = usage.typed_ast_record_count.saturating_add(count as u64);
    }
    for v in &value.evidence {
        usage.relation_occurrence_count = usage
            .relation_occurrence_count
            .saturating_add(3 + u64::from(v.description.is_some()));
        usage.charge_table(3, 12);
        usage.charge_token(&v.key, limits)?;
        usage.charge_token(&v.locator, limits)?;
        if let Some(d) = &v.description {
            usage.charge_token(d, limits)?;
        }
    }
    for v in &value.gaps {
        usage.relation_occurrence_count = usage.relation_occurrence_count.saturating_add(6);
        usage.charge_table(5, 32);
        usage.charge_token(&v.key, limits)?;
        usage.charge_token(&v.parameter_version, limits)?;
    }
    for v in &value.streams {
        usage.relation_occurrence_count = usage.relation_occurrence_count.saturating_add(
            6 + u64::from(v.classes.is_some())
                + u64::from(v.gap.is_some())
                + v.classes.as_ref().map_or(0, |classes| classes.len()) as u64
                + v.yield_to.len() as u64
                + v.evidence.len() as u64,
        );
        usage.charge_table(7, 28);
        usage.charge_token(&v.key, limits)?;
        usage.charge_reference(&v.stream, namespace, imports, limits)?;
        if let Some(classes) = &v.classes {
            usage.charge_vector(classes.len(), 4);
            for r in classes {
                usage.charge_reference(r, namespace, imports, limits)?;
            }
        }
        usage.charge_vector(v.yield_to.len(), 4);
        for r in &v.yield_to {
            usage.charge_reference(r, namespace, imports, limits)?;
        }
        if let Some(gap) = &v.gap {
            usage.charge_token(gap, limits)?;
        }
        usage.charge_vector(v.evidence.len(), 4);
        for key in &v.evidence {
            usage.charge_token(key, limits)?;
        }
    }
    for v in &value.gates {
        usage.relation_occurrence_count = usage.relation_occurrence_count.saturating_add(
            6 + u64::from(v.classes.is_some())
                + v.classes.as_ref().map_or(0, |classes| classes.len()) as u64
                + v.evidence.len() as u64,
        );
        usage.charge_table(6, 18);
        usage.charge_token(&v.key, limits)?;
        usage.charge_reference(&v.gate, namespace, imports, limits)?;
        if let Some(classes) = &v.classes {
            usage.charge_vector(classes.len(), 4);
            for r in classes {
                usage.charge_reference(r, namespace, imports, limits)?;
            }
        }
        usage.charge_vector(v.evidence.len(), 4);
        for key in &v.evidence {
            usage.charge_token(key, limits)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::road_editing::{RoadEditingModuleInput, RoadEditingSourceWriter};

    #[test]
    fn movement_direction_charges_logical_records_and_relations_before_admission() {
        let limits = CompileLimits::p100_initial_v1();
        fn new_builder(limits: &CompileLimits) -> RoadEditingSourceModuleBuilder<'_> {
            let mut builder = RoadEditingSourceModuleBuilder::new(
                RoadEditingModuleHeader::try_new(
                    "city",
                    "directions",
                    vec![],
                    RoadEditingProvenance::direct("direction-budget").unwrap(),
                )
                .unwrap(),
                GeometryAccuracyProfile::Balanced5Cm,
                GeometryDirectionProfile::Balanced2Deg,
                limits,
            )
            .unwrap();
            builder
                .add_declaration(RoadEditingDeclaration::Junction(
                    JunctionInput::try_new(
                        "junction",
                        vec![LaneEdgeReference::local("entry").unwrap()],
                        vec![],
                    )
                    .unwrap(),
                ))
                .unwrap();
            builder
        }
        let movement = || {
            MovementInput::try_new(
                "movement",
                JunctionReference::local("junction").unwrap(),
                "from",
                "to",
            )
            .unwrap()
        };
        for direction in [
            crate::ManeuverDirection::Straight,
            crate::ManeuverDirection::Right,
        ] {
            let exact = limits
                .clone()
                .with_test_admission_limit(CompileLimitDimension::TypedAstRecordCount, 4)
                .with_test_admission_limit(CompileLimitDimension::RelationOccurrenceCount, 2);
            let mut builder = new_builder(&exact);
            builder
                .add_declaration(RoadEditingDeclaration::Movement(
                    movement().with_turn_direction(direction),
                ))
                .unwrap();
            assert_eq!(builder.usage.typed_ast_record_count, 4);
            assert_eq!(builder.usage.relation_occurrence_count, 2);
            let source = RoadEditingSourceWriter::new(&exact)
                .write(builder.finish().unwrap())
                .unwrap();
            let input =
                RoadEditingModuleInput::try_new("directions", source.as_bytes(), None).unwrap();
            let checked = super::super::super::reader::verify_source(input, &exact, 0, 0).unwrap();
            assert_eq!(checked.table_count(), 5);
            assert_eq!(checked.typed_ast_record_count(), 4);
            assert_eq!(checked.preflight_counts().relation_occurrence_count(), 2);
            assert!(super::super::super::reader::verify_source(input, &exact, 0, 1).is_err());
            for (dimension, limit) in [
                (CompileLimitDimension::TypedAstRecordCount, 3),
                (CompileLimitDimension::RelationOccurrenceCount, 1),
            ] {
                let low = limits.clone().with_test_admission_limit(dimension, limit);
                let mut retry = new_builder(&low);
                assert!(
                    retry
                        .add_declaration(RoadEditingDeclaration::Movement(
                            movement().with_turn_direction(direction)
                        ))
                        .is_err()
                );
                assert_eq!(retry.usage.typed_ast_record_count, 2);
                assert_eq!(retry.usage.relation_occurrence_count, 1);
                retry
                    .add_declaration(RoadEditingDeclaration::Movement(movement()))
                    .unwrap();
                let error =
                    super::super::super::reader::verify_source(input, &low, 0, 0).unwrap_err();
                assert!(
                    matches!(error.diagnostics()[0].payload(), crate::DiagnosticPayload::CompileLimitExceeded { dimension: found, .. } if *found == dimension)
                );
                let small = RoadEditingSourceWriter::new(&low)
                    .write(retry.finish().unwrap())
                    .unwrap();
                let small =
                    RoadEditingModuleInput::try_new("directions", small.as_bytes(), None).unwrap();
                assert_eq!(
                    super::super::super::reader::verify_source(small, &low, 0, 0)
                        .unwrap()
                        .typed_ast_record_count(),
                    3
                );
            }
        }
    }

    #[test]
    fn policy_reference_members_share_builder_and_raw_relation_limits() {
        let limits = CompileLimits::p100_initial_v1();
        let mut policy = None;
        crate::compiler::policy_tests::editing_policy_custom(&limits, |_, value| {
            for rule in &mut value.streams {
                rule.classes =
                    Some(vec![ParticipantClassReference::local("vehicle").unwrap()].into());
            }
            for rule in &mut value.gates {
                rule.classes =
                    Some(vec![ParticipantClassReference::local("vehicle").unwrap()].into());
            }
            policy = Some(value.clone());
        });
        let policy = policy.unwrap();
        let new_builder = |limits| {
            RoadEditingSourceModuleBuilder::new(
                RoadEditingModuleHeader::try_new(
                    "policy",
                    "policy.document",
                    vec!["city/portable-full-spatial-conflict".into()],
                    RoadEditingProvenance::direct("policy-relations").unwrap(),
                )
                .unwrap(),
                GeometryAccuracyProfile::Balanced5Cm,
                GeometryDirectionProfile::Balanced2Deg,
                limits,
            )
            .unwrap()
        };
        let exact = limits
            .clone()
            .with_test_admission_limit(CompileLimitDimension::RelationOccurrenceCount, 50);
        let low = limits
            .clone()
            .with_test_admission_limit(CompileLimitDimension::RelationOccurrenceCount, 49);
        let mut good = new_builder(&exact);
        good.add_declaration(RoadEditingDeclaration::RightOfWayPolicySet(policy.clone()))
            .unwrap();
        // 规则字段/来源、依据、四个 selector 成员与一个 yield 成员共 50 条关系。
        assert_eq!(good.usage.relation_occurrence_count, 50);
        let source = RoadEditingSourceWriter::new(&exact)
            .write(good.finish().unwrap())
            .unwrap();
        let input =
            RoadEditingModuleInput::try_new("policy.document", source.as_bytes(), None).unwrap();
        let checked = super::super::super::reader::verify_source(input, &exact, 0, 0).unwrap();
        assert_eq!(checked.preflight_counts().relation_occurrence_count(), 50);
        assert!(super::super::super::reader::verify_source(input, &low, 0, 0).is_err());
        let mut rejected = new_builder(&low);
        assert!(
            rejected
                .add_declaration(RoadEditingDeclaration::RightOfWayPolicySet(policy.clone()))
                .is_err()
        );
        assert_eq!(rejected.usage.relation_occurrence_count, 0);
        let mut smaller = policy;
        for rule in &mut smaller.streams {
            rule.classes = None;
        }
        for rule in &mut smaller.gates {
            rule.classes = None;
        }
        rejected
            .add_declaration(RoadEditingDeclaration::RightOfWayPolicySet(smaller))
            .unwrap();
        assert_eq!(rejected.usage.relation_occurrence_count, 42);
    }
}
