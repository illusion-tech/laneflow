use super::*;
use crate::declaration::{
    PolicyDeclarationSource, PolicyEvidenceDeclaration, PolicyGapProfileDeclaration,
    PolicyGateRuleDeclaration, PolicyStreamRuleDeclaration, RightOfWayPolicySetDeclaration,
};
use crate::{DiagnosticBundle, GateInterpretation, GateProhibition, RegulationIdentity};
use laneflow_static_contract::ParticipantStreamKind;

fn step(table: RoadEditingTableKind, field_id: u16) -> RoadEditingPropertyStep {
    RoadEditingPropertyStep::TableField { table, field_id }
}

struct Locations<'a> {
    factory: &'a RoadEditingLocationFactory,
    key: &'a str,
    canvas: Option<&'a str>,
}
impl Locations<'_> {
    fn declaration(&self, steps: &[RoadEditingPropertyStep]) -> SourceLocation {
        self.factory.property(
            EntityKind::RightOfWayPolicySet,
            &[],
            self.key,
            steps,
            self.canvas,
        )
    }
    fn member(
        &self,
        field: u16,
        table: RoadEditingTableKind,
        relation: RoadEditingRelationKind,
        index: usize,
        fields: &[u16],
    ) -> PolicyDeclarationSource {
        let location = |steps: &[RoadEditingPropertyStep]| {
            self.factory.owner_local(
                EntityKind::RightOfWayPolicySet,
                &[],
                self.key,
                relation,
                RoadEditingRelationOccurrence::CanonicalSetOrdinal(
                    u32::try_from(index).expect("admitted member count"),
                ),
                steps,
                self.canvas,
            )
        };
        let outer = step(RoadEditingTableKind::RightOfWayPolicySet, field);
        PolicyDeclarationSource {
            primary: location(&[outer]),
            contributing: fields
                .iter()
                .map(|&field_id| location(&[outer, step(table, field_id)]))
                .collect(),
        }
    }
}

fn evidence_keys(values: runtime::Vector<'_, runtime::ForwardsUOffset<&str>>) -> Box<[Arc<str>]> {
    let mut values: Box<[Arc<str>]> = values.iter().map(Arc::from).collect();
    values.sort_unstable();
    values
}
fn classes(
    values: Option<runtime::Vector<'_, runtime::ForwardsUOffset<&str>>>,
    namespace: &Arc<str>,
    source: &SourceLocation,
) -> Option<Box<[OwnedEntityReference<ParticipantClassKind>]>> {
    values.map(|values| {
        let mut values: Box<[_]> = values
            .iter()
            .map(|v| lower_reference(v, 1, namespace, source.clone()))
            .collect();
        crate::policy::sort_references(&mut values);
        values
    })
}

pub(in crate::road_editing) fn lower(
    root: wire::RoadEditingSource<'_>,
    factory: &RoadEditingLocationFactory,
    namespace: &Arc<str>,
    declarations: &mut Vec<TypedAstDeclaration>,
) -> Result<(), DiagnosticBundle> {
    for value in root.right_of_way_policy_sets() {
        let locations = Locations {
            factory,
            key: value.policy_set_key(),
            canvas: value.canvas_selection(),
        };
        let regulation = value.regulation();
        let mut contributing =
            vec![locations.declaration(&[step(RoadEditingTableKind::RightOfWayPolicySet, 0)])];
        for field in 0..(2 + u16::from(regulation.source().is_some())) {
            contributing.push(locations.declaration(&[
                step(RoadEditingTableKind::RightOfWayPolicySet, 1),
                step(RoadEditingTableKind::AccessRegulation, field),
            ]));
        }
        // 一次只保留一个成员种类的借用排序视图；容量已由 admission 预收。
        let evidence = {
            let mut order: Vec<_> = value.evidence().iter().collect();
            order.sort_unstable_by_key(|v| v.evidence_key());
            order
                .into_iter()
                .enumerate()
                .map(|(index, v)| PolicyEvidenceDeclaration {
                    key: Arc::from(v.evidence_key()),
                    locator: Arc::from(v.locator()),
                    description: v.description().map(Arc::from),
                    source: locations.member(
                        2,
                        RoadEditingTableKind::PolicyEvidence,
                        RoadEditingRelationKind::PolicyEvidence,
                        index,
                        if v.description().is_some() {
                            &[0, 1, 2]
                        } else {
                            &[0, 1]
                        },
                    ),
                })
                .collect()
        };
        let gap_profiles = {
            let mut order: Vec<_> = value.gap_profiles().iter().collect();
            order.sort_unstable_by_key(|v| v.profile_key());
            order
                .into_iter()
                .enumerate()
                .map(|(index, v)| PolicyGapProfileDeclaration {
                    key: Arc::from(v.profile_key()),
                    parameter_version: Arc::from(v.parameter_version()),
                    minimum_lead_gap_ms: v.minimum_lead_gap_ms().expect("preflight"),
                    minimum_lag_gap_ms: v.minimum_lag_gap_ms().expect("preflight"),
                    clearance_buffer_ms: v.clearance_buffer_ms().expect("preflight"),
                    source: locations.member(
                        3,
                        RoadEditingTableKind::PolicyGapProfile,
                        RoadEditingRelationKind::PolicyGapProfile,
                        index,
                        &[0, 1, 2, 3, 4],
                    ),
                })
                .collect()
        };
        let stream_rules = {
            let mut order: Vec<_> = value.stream_rules().iter().collect();
            order.sort_unstable_by_key(|v| v.rule_key());
            order
                .into_iter()
                .enumerate()
                .map(|(index, v)| {
                    let all_fields = [0, 1, 2, 3, 4, 5, 6];
                    let mut fields = [0; 7];
                    let mut count = 0;
                    for field in all_fields {
                        if (field != 2 || v.participant_classes().is_some())
                            && (field != 5 || v.gap_profile_key().is_some())
                        {
                            fields[count] = field;
                            count += 1;
                        }
                    }
                    let source = locations.member(
                        4,
                        RoadEditingTableKind::PolicyStreamRule,
                        RoadEditingRelationKind::PolicyStreamRule,
                        index,
                        &fields[..count],
                    );
                    let field_source = |field| {
                        source.contributing[fields[..count]
                            .iter()
                            .position(|&f| f == field)
                            .expect("present field")]
                        .clone()
                    };
                    let mut yield_to: Box<[OwnedEntityReference<ParticipantStreamKind>]> = v
                        .yield_to_streams()
                        .iter()
                        .map(|r| lower_reference(r, 2, namespace, field_source(4)))
                        .collect();
                    crate::policy::sort_references(&mut yield_to);
                    PolicyStreamRuleDeclaration {
                        key: Arc::from(v.rule_key()),
                        stream: lower_reference(v.stream(), 2, namespace, field_source(1)),
                        classes: v.participant_classes().map(|_| {
                            classes(v.participant_classes(), namespace, &field_source(2))
                                .expect("present")
                        }),
                        priority: v.priority().expect("preflight"),
                        yield_to,
                        gap: v.gap_profile_key().map(Arc::from),
                        evidence: evidence_keys(v.evidence_keys()),
                        source,
                    }
                })
                .collect()
        };
        let gate_rules = {
            let mut order: Vec<_> = value.gate_rules().iter().collect();
            order.sort_unstable_by_key(|v| v.rule_key());
            order
                .into_iter()
                .enumerate()
                .map(|(index, v)| {
                    let fields: &[u16] = if v.participant_classes().is_some() {
                        &[0, 1, 2, 3, 4, 5]
                    } else {
                        &[0, 1, 3, 4, 5]
                    };
                    let source = locations.member(
                        5,
                        RoadEditingTableKind::PolicyGateRule,
                        RoadEditingRelationKind::PolicyGateRule,
                        index,
                        fields,
                    );
                    PolicyGateRuleDeclaration {
                        key: Arc::from(v.rule_key()),
                        gate: lower_reference(
                            v.gate(),
                            4,
                            namespace,
                            source.contributing[1].clone(),
                        ),
                        classes: v.participant_classes().map(|_| {
                            classes(v.participant_classes(), namespace, &source.contributing[2])
                                .expect("present")
                        }),
                        interpretation: GateInterpretation::from_code(
                            v.interpretation().expect("preflight").0,
                        )
                        .expect("preflight"),
                        prohibition: GateProhibition::from_code(
                            v.prohibition().expect("preflight").0,
                        )
                        .expect("preflight"),
                        evidence: evidence_keys(v.evidence_keys()),
                        source,
                    }
                })
                .collect()
        };
        let declaration = RightOfWayPolicySetDeclaration {
            header: DeclarationHeader::module_scoped(
                EntityKind::RightOfWayPolicySet,
                Arc::from(value.policy_set_key()),
                factory.declaration(
                    EntityKind::RightOfWayPolicySet,
                    &[],
                    value.policy_set_key(),
                    value.canvas_selection(),
                ),
            ),
            regulation: RegulationIdentity {
                jurisdiction: Arc::from(regulation.jurisdiction()),
                version: Arc::from(regulation.version()),
                source: regulation.source().map(Arc::from),
            },
            evidence,
            gap_profiles,
            stream_rules,
            gate_rules,
            contributing: contributing.into_boxed_slice(),
        };
        crate::policy::validate_local_declaration(&declaration)?;
        declarations.push(TypedAstDeclaration::RightOfWayPolicySet(declaration));
    }
    Ok(())
}
