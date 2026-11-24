use super::*;

/// 为一套通行权策略集合输入向模块用量计费，并在校验失败时返回诊断。
pub(super) fn charge(
    usage: &mut ModuleUsage,
    value: &RightOfWayPolicySetInput,
    namespace: &str,
    imports: &BTreeSet<Box<str>>,
    limits: &CompileProfile,
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
        let limits = CompileProfile::single_network_10k_stable_entities();
        fn new_builder(limits: &CompileProfile) -> RoadEditingSourceModuleBuilder<'_> {
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
            let checked =
                super::super::super::reader::verify_source(input, &exact, 0, 0, 0).unwrap();
            assert_eq!(checked.table_count(), 5);
            assert_eq!(checked.typed_ast_record_count(), 4);
            assert_eq!(checked.preflight_counts().relation_occurrence_count(), 2);
            assert!(super::super::super::reader::verify_source(input, &exact, 0, 1, 0).is_err());
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
                    super::super::super::reader::verify_source(input, &low, 0, 0, 0).unwrap_err();
                assert!(
                    matches!(error.diagnostics()[0].payload(), crate::DiagnosticPayload::CompileLimitExceeded { dimension: found, .. } if *found == dimension)
                );
                let small = RoadEditingSourceWriter::new(&low)
                    .write(retry.finish().unwrap())
                    .unwrap();
                let small =
                    RoadEditingModuleInput::try_new("directions", small.as_bytes(), None).unwrap();
                assert_eq!(
                    super::super::super::reader::verify_source(small, &low, 0, 0, 0)
                        .unwrap()
                        .typed_ast_record_count(),
                    3
                );
            }
        }
    }

    #[test]
    fn policy_reference_members_share_builder_and_raw_relation_limits() {
        let limits = CompileProfile::single_network_10k_stable_entities();
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
        let checked = super::super::super::reader::verify_source(input, &exact, 0, 0, 0).unwrap();
        assert_eq!(checked.preflight_counts().relation_occurrence_count(), 50);
        assert!(super::super::super::reader::verify_source(input, &low, 0, 0, 0).is_err());
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

    #[test]
    fn many_unique_empty_policy_sets_are_admitted_by_real_relation_count() {
        // 461 个策略集的两两比较数为 106030，超过常规档来源关系上限 105967；预检只按
        // 真实字段关系计数（每个无法规来源的空策略集 3 条），去重使用受预算排序。
        const POLICY_SETS: usize = 461;
        const RELATIONS: u64 = 3 * POLICY_SETS as u64;
        let limits = CompileProfile::single_network_10k_stable_entities();
        assert!(
            (POLICY_SETS * (POLICY_SETS - 1) / 2) as u64
                > limits
                    .source_limits()
                    .get(crate::SourceProcessingDimension::RelationOccurrenceCount)
        );
        let source_with = |limits: &CompileProfile| {
            let mut builder = RoadEditingSourceModuleBuilder::new(
                RoadEditingModuleHeader::try_new(
                    "policy",
                    "policies",
                    vec![],
                    RoadEditingProvenance::direct("many-policy-sets").unwrap(),
                )
                .unwrap(),
                GeometryAccuracyProfile::Balanced5Cm,
                GeometryDirectionProfile::Balanced2Deg,
                limits,
            )
            .unwrap();
            for index in 0..POLICY_SETS {
                builder
                    .add_declaration(RoadEditingDeclaration::RightOfWayPolicySet(
                        RightOfWayPolicySetInput::try_new(
                            format!("policy-{index:03}"),
                            crate::RegulationIdentity::try_new("XX", "2026").unwrap(),
                            vec![],
                            vec![],
                            vec![],
                            vec![],
                        )
                        .unwrap(),
                    ))
                    .unwrap();
            }
            assert_eq!(builder.usage.relation_occurrence_count, RELATIONS);
            RoadEditingSourceWriter::new(limits)
                .write(builder.finish().unwrap())
                .unwrap()
        };
        let source = source_with(&limits);
        let input = RoadEditingModuleInput::try_new("policies", source.as_bytes(), None).unwrap();
        let checked = super::super::super::reader::verify_source(input, &limits, 0, 0, 0).unwrap();
        assert_eq!(
            checked.preflight_counts().relation_occurrence_count(),
            RELATIONS
        );

        // 真实关系计数仍按上限准入：达界成功，少一按来源关系维度拒绝，之后同一输入可复用。
        let exact = limits.clone().with_test_admission_limit(
            CompileLimitDimension::RelationOccurrenceCount,
            RELATIONS as u32,
        );
        let low = limits.clone().with_test_admission_limit(
            CompileLimitDimension::RelationOccurrenceCount,
            RELATIONS as u32 - 1,
        );
        assert!(super::super::super::reader::verify_source(input, &exact, 0, 0, 0).is_ok());
        let error = super::super::super::reader::verify_source(input, &low, 0, 0, 0).unwrap_err();
        assert!(matches!(
            error.diagnostics()[0].payload(),
            crate::DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::RelationOccurrenceCount,
                ..
            }
        ));
        assert!(super::super::super::reader::verify_source(input, &exact, 0, 0, 0).is_ok());

        // 去重仍由受预算排序完成：把最后一个键改成前一个键按重复拒绝；操作内存预算不足时
        // 按该维度拒绝。两种拒绝后原输入在正式政策下仍可通过。
        let mut duplicate = source.as_bytes().to_vec();
        let offset = {
            let root = laneflow_road_editing_wire::generated::lane_flow::road_editing::v1::size_prefixed_root_as_road_editing_source(&duplicate)
                .unwrap();
            let key = root
                .right_of_way_policy_sets()
                .iter()
                .map(|policy| policy.policy_set_key())
                .find(|key| *key == "policy-460")
                .unwrap();
            key.as_ptr() as usize - duplicate.as_ptr() as usize
        };
        duplicate[offset..offset + "policy-460".len()].copy_from_slice(b"policy-459");
        let duplicate = RoadEditingModuleInput::try_new("policies", &duplicate, None).unwrap();
        let error =
            super::super::super::reader::verify_source(duplicate, &limits, 0, 0, 0).unwrap_err();
        assert!(matches!(
            error.diagnostics()[0].payload(),
            crate::DiagnosticPayload::InvalidRoadEditingSource {
                violation: crate::RoadEditingSourceViolation::InvalidSemanticValue(
                    crate::RoadEditingInputViolation::DuplicateValue
                ),
                field: Some(field),
                ..
            } if field.as_ref() == "rightOfWayPolicySets.policySetKey"
        ));
        let starved = limits
            .clone()
            .with_test_admission_limit(CompileLimitDimension::CompilerControlledLiveBytes, 0);
        let error =
            super::super::super::reader::verify_source(input, &starved, 0, 0, 0).unwrap_err();
        assert!(matches!(
            error.diagnostics()[0].payload(),
            crate::DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::CompilerControlledLiveBytes,
                ..
            }
        ));
        assert!(super::super::super::reader::verify_source(input, &limits, 0, 0, 0).is_ok());
    }
}
