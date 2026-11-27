use super::*;

fn input(classes: &[&str], rules: &[(&str, &[&str])]) -> CompilationUnit {
    let mut builder = access_builder("access-members.document");
    for &key in classes {
        builder
            .add_participant_class(ParticipantClassInput {
                participant_class_key: key,
                extends: None,
            })
            .unwrap();
    }
    for &(key, members) in rules {
        let references: Vec<_> = members
            .iter()
            .map(|&member| ParticipantClassReference::local(member))
            .collect();
        builder
            .add_access_rule(AccessRuleInput {
                access_rule_key: key,
                target: AccessRuleTargetInput::LaneEdge(LaneEdgeReference::local("edge-main")),
                effect: AccessEffect::Allow,
                participant_classes: &references,
                regulation: None,
                priority: 0,
            })
            .unwrap();
    }
    unit([builder.finish().unwrap()])
}

#[test]
fn each_access_rule_owns_its_ordered_range_of_distinct_classes() {
    let unique = input(
        &["car", "bus", "truck"],
        &[
            ("rule-a", &["truck", "bus", "car"]),
            ("rule-b", &["car", "truck"]),
        ],
    );
    let permuted_with_repeats = input(
        &["truck", "bus", "car"],
        &[
            ("rule-b", &["truck", "car"]),
            ("rule-a", &["car", "bus", "car", "truck", "bus"]),
        ],
    );
    let digests = [unique, permuted_with_repeats].map(|input| {
        let hir = build_hir(&input).unwrap();
        let members: Vec<_> = hir
            .access_rule_participant_classes
            .iter()
            .map(|member| {
                hir.participant_classes[member.participant_class.index()]
                    .stable_key
                    .as_ref()
            })
            .collect();
        assert_eq!(members, ["bus", "car", "truck", "car", "truck"]);
        let ranges: Vec<_> = hir
            .access_rules
            .iter()
            .map(|rule| {
                (
                    rule.stable_key.as_ref(),
                    rule.participant_classes.as_usize_range(),
                )
            })
            .collect();
        assert_eq!(ranges, [("rule-a", 0..3), ("rule-b", 3..5)]);
        Compiler::new()
            .compile(input)
            .unwrap()
            .lir
            .inner
            .semantic_digest
    });
    assert_eq!(digests[0], digests[1]);
}

#[test]
fn empty_access_rule_members_report_sorted_payloads_and_allow_retry() {
    let mut compiler = Compiler::new();
    let failures = [
        input(
            &["car"],
            &[("rule-z", &[]), ("rule-ok", &["car"]), ("rule-a", &[])],
        ),
        input(
            &["car"],
            &[("rule-a", &[]), ("rule-ok", &["car"]), ("rule-z", &[])],
        ),
    ]
    .map(|input| {
        let Err(bundle) = compiler.compile(input) else {
            panic!("empty access rule members must be rejected");
        };
        assert!(!bundle.diagnostics_truncated());
        assert_eq!(bundle.diagnostics().len(), 2);
        for (diagnostic, key) in bundle.diagnostics().iter().zip(["rule-a", "rule-z"]) {
            assert_eq!(
                diagnostic.code(),
                DiagnosticCode::EmptyAccessRuleParticipantClasses
            );
            assert_eq!(
                diagnostic.payload(),
                &DiagnosticPayload::EmptyAccessRuleParticipantClasses {
                    access_rule_key: key.into(),
                }
            );
            assert_eq!(diagnostic.stable_key(), Some(key));
            assert_eq!(
                diagnostic.primary_span().unwrap().source_document_key(),
                "access-members.document"
            );
        }
        bundle
    });
    assert_eq!(failures[0], failures[1]);
    let output = compiler
        .compile(input(&["car"], &[("rule-ok", &["car"])]))
        .unwrap();
    assert!(output.diagnostics().is_empty());
    assert_eq!(output.lir().access_rules().len(), 1);
}
