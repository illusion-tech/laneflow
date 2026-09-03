use super::*;

fn primary<T>(result: Result<T, DiagnosticBundle>, expected: PolicyViolation) -> SourceLocation {
    let errors = result.err().expect("invalid policy must fail");
    errors
        .diagnostics()
        .iter()
        .find(|diagnostic| {
            matches!(diagnostic.payload(), DiagnosticPayload::InvalidPolicy { violation, .. } if *violation == expected)
        })
        .expect("expected policy violation")
        .primary_location()
        .expect("member location")
        .clone()
}

#[test]
fn synthetic_local_errors_preserve_the_failing_rule_text_span() {
    static STREAM: std::sync::LazyLock<SourceSpan> =
        std::sync::LazyLock::new(|| SourceSpan::point(DOCUMENT.into(), 20, 4));
    static GATE: std::sync::LazyLock<SourceSpan> =
        std::sync::LazyLock::new(|| SourceSpan::point(DOCUMENT.into(), 30, 7));
    let limits = CompileLimits::p100_initial_v1();
    for gate in [false, true] {
        for (case, violation) in [
            PolicyViolation::MissingEvidence,
            PolicyViolation::MissingLocalReference,
            PolicyViolation::DuplicateReference,
            PolicyViolation::DuplicateMember,
            PolicyViolation::EmptyClasses,
            PolicyViolation::InvalidKey,
            PolicyViolation::MissingLocalReference,
            PolicyViolation::GapBinding,
        ]
        .into_iter()
        .enumerate()
        {
            if gate && case >= 6 {
                continue;
            }
            let result = synthetic_policy(&limits, |streams, gates| {
                streams[0].source.primary = &STREAM;
                gates[0].source.primary = &GATE;
                if gate {
                    match case {
                        0 => gates[0].evidence_keys = &[],
                        1 => gates[0].evidence_keys = &["missing"],
                        2 => gates[0].evidence_keys = &["basis", "basis"],
                        3 => gates.push(gates[0]),
                        4 => gates[0].participant_classes = Some(&[]),
                        5 => gates[0].rule_key = "invalid::key",
                        _ => unreachable!(),
                    }
                } else {
                    match case {
                        0 => streams[0].evidence_keys = &[],
                        1 => streams[0].evidence_keys = &["missing"],
                        2 => streams[0].evidence_keys = &["basis", "basis"],
                        3 => streams.push(streams[0]),
                        4 => streams[0].participant_classes = Some(&[]),
                        5 => streams[0].rule_key = "invalid::key",
                        6 => streams[0].gap_profile_key = Some("missing"),
                        7 => streams[0].gap_profile_key = None,
                        _ => unreachable!(),
                    }
                }
            });
            let expected = if gate { &*GATE } else { &*STREAM };
            assert_eq!(primary(result, violation), expected.clone().into());
        }
    }
}

#[test]
fn road_editing_local_errors_preserve_the_failing_rule_owner_local_location() {
    for gate in [false, true] {
        for (case, violation) in [
            PolicyViolation::MissingEvidence,
            PolicyViolation::MissingLocalReference,
            PolicyViolation::DuplicateReference,
            PolicyViolation::DuplicateMember,
            PolicyViolation::MissingLocalReference,
        ]
        .into_iter()
        .enumerate()
        {
            if gate && case == 4 {
                continue;
            }
            let result = unit_editing_custom(None, None, |_, policy| {
                if gate {
                    match case {
                        0 => policy.gates[0].evidence = Box::new([]),
                        1 => policy.gates[0].evidence = Box::new(["missing".into()]),
                        2 => policy.gates[0].evidence = Box::new(["basis".into(), "basis".into()]),
                        3 => {
                            let mut rules = policy.gates.to_vec();
                            rules.push(rules[0].clone());
                            policy.gates = rules.into_boxed_slice();
                        }
                        _ => unreachable!(),
                    }
                } else {
                    match case {
                        0 => policy.streams[0].evidence = Box::new([]),
                        1 => policy.streams[0].evidence = Box::new(["missing".into()]),
                        2 => {
                            policy.streams[0].evidence = Box::new(["basis".into(), "basis".into()])
                        }
                        3 => {
                            let mut rules = policy.streams.to_vec();
                            rules.push(rules[0].clone());
                            policy.streams = rules.into_boxed_slice();
                        }
                        4 => policy.streams[0].gap = Some("missing".into()),
                        _ => unreachable!(),
                    }
                }
            });
            let SourceLocation::RoadEditing(location) = primary(result, violation) else {
                panic!("expected editor location");
            };
            let relation = if gate {
                RoadEditingRelationKind::PolicyGateRule
            } else {
                RoadEditingRelationKind::PolicyStreamRule
            };
            let ordinal = u32::from(case == 3);
            assert!(
                matches!(location.subject(), RoadEditingSubject::OwnerLocal {
                relation: actual,
                occurrence: RoadEditingRelationOccurrence::CanonicalSetOrdinal(actual_ordinal),
                ..
            } if *actual == relation && *actual_ordinal == ordinal)
            );
            assert_eq!(
                location.property_path().unwrap().steps(),
                &[RoadEditingPropertyStep::TableField {
                    table: RoadEditingTableKind::RightOfWayPolicySet,
                    field_id: if gate { 5 } else { 4 },
                }]
            );
        }
    }
}
