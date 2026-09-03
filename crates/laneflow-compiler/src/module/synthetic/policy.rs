//! 完整策略声明的有界原子准入。
use super::*;
use crate::declaration::*;
use crate::{OwnerQualifiedReference, PolicyViolation as V, RightOfWayPolicySetInput};
use laneflow_static_contract::EntityKindMarker;

#[derive(Default)]
struct Sizing {
    strings: u64,
    bytes: u64,
    max_string: u64,
    references: u64,
    locations: u64,
    source: u64,
    structural: u64,
}
impl Sizing {
    fn text(&mut self, value: &str) {
        self.strings = self.strings.saturating_add(1);
        self.bytes = self.bytes.saturating_add(value.len() as u64);
        self.max_string = self.max_string.max(value.len() as u64);
    }
    fn reference<K: EntityKindMarker>(&mut self, value: EntityReference<'_, K>, namespace: &str) {
        self.references = self.references.saturating_add(1);
        self.text(value.module_namespace().unwrap_or(namespace));
        self.text(value.declaration_key());
        self.structural = self
            .structural
            .saturating_add(size_bytes::<OwnedEntityReference<K>>(1));
        self.locations = self.locations.saturating_add(1);
    }
    fn qualified<K: EntityKindMarker>(
        &mut self,
        value: OwnerQualifiedReference<'_, K>,
        namespace: &str,
    ) {
        self.reference(value.target, namespace);
        for key in value.owner_keys {
            self.text(key);
        }
        self.structural = self
            .structural
            .saturating_add(size_bytes::<Arc<str>>(value.owner_keys.len() as u64))
            .saturating_add(2 * size_of::<usize>() as u64);
    }
    fn source(&mut self, source: PolicyInputSource<'_>) {
        self.locations = self
            .locations
            .saturating_add(1)
            .saturating_add(source.contributing.len() as u64);
        self.structural = self.structural.saturating_add(size_bytes::<SourceLocation>(
            source.contributing.len() as u64,
        ));
    }
}

impl SyntheticModuleBuilder {
    /// 加入由一个模块拥有的完整策略。所有检查成功后才修改 builder。
    pub fn add_right_of_way_policy_set(
        &mut self,
        input: RightOfWayPolicySetInput<'_>,
    ) -> Result<&mut Self, DiagnosticBundle> {
        let span = input.source.primary;
        let key = input.policy_set_key;
        let fail = |member: Option<&str>, violation, span: &SourceSpan| {
            crate::policy::error(key, member, violation, &span.clone().into())
        };
        self.validate_declaration_key(EntityKind::RightOfWayPolicySet, key, span)?;
        if input.regulation.validate().is_err() {
            return Err(fail(None, V::InvalidRegulation, span));
        }
        let mut size = Sizing::default();
        size.text(key);
        size.text(&self.header.authoring_namespace_id);
        size.text(input.regulation.jurisdiction);
        size.text(input.regulation.version);
        if let Some(source) = input.regulation.source {
            size.text(source);
        }
        let check_source = |source: PolicyInputSource<'_>,
                            member: Option<&str>|
         -> Result<(), DiagnosticBundle> {
            if core::iter::once(source.primary)
                .chain(source.contributing)
                .any(|span| span.source_document_key() != self.header.source_document_key.as_ref())
            {
                return Err(fail(member, V::SourceDocument, source.primary));
            }
            Ok(())
        };
        let check_key = |value: &str, source: &SourceSpan| -> Result<(), DiagnosticBundle> {
            if external_token_violation(value, self.limits.identity_ascii_bytes_limit()).is_some()
                || value.contains("::")
            {
                return Err(fail(Some(value), V::InvalidKey, source));
            }
            Ok(())
        };
        check_source(input.source, None)?;
        size.source(input.source);
        for member in input.evidence {
            check_key(member.evidence_key, member.source.primary)?;
            check_source(member.source, Some(member.evidence_key))?;
            if member.locator.is_empty() {
                return Err(fail(
                    Some(member.evidence_key),
                    V::EmptyValue,
                    member.source.primary,
                ));
            }
            size.text(member.evidence_key);
            size.text(member.locator);
            if let Some(description) = member.description {
                size.text(description);
            }
            size.source(member.source);
        }
        for member in input.gap_profiles {
            check_key(member.profile_key, member.source.primary)?;
            check_source(member.source, Some(member.profile_key))?;
            if member.parameter_version.is_empty() {
                return Err(fail(
                    Some(member.profile_key),
                    V::EmptyValue,
                    member.source.primary,
                ));
            }
            size.text(member.profile_key);
            size.text(member.parameter_version);
            size.source(member.source);
        }
        for member in input.stream_rules {
            check_key(member.rule_key, member.source.primary)?;
            check_source(member.source, Some(member.rule_key))?;
            self.validate_policy_reference(member.stream, member.source.primary)?;
            if member.participant_classes.is_some_and(<[_]>::is_empty) {
                return Err(fail(
                    Some(member.rule_key),
                    V::EmptyClasses,
                    member.source.primary,
                ));
            }
            if member.yield_to_streams.is_empty() != member.gap_profile_key.is_none() {
                return Err(fail(
                    Some(member.rule_key),
                    V::GapBinding,
                    member.source.primary,
                ));
            }
            size.text(member.rule_key);
            size.qualified(member.stream, &self.header.authoring_namespace_id);
            size.source(member.source);
            for reference in member.participant_classes.unwrap_or(&[]) {
                self.validate_reference(
                    EntityKind::ParticipantClass,
                    *reference,
                    member.source.primary,
                )?;
                size.reference(*reference, &self.header.authoring_namespace_id);
            }
            for reference in member.yield_to_streams {
                self.validate_policy_reference(*reference, member.source.primary)?;
                size.qualified(*reference, &self.header.authoring_namespace_id);
            }
            if let Some(gap) = member.gap_profile_key {
                check_key(gap, member.source.primary)?;
                size.text(gap);
            }
            for evidence in member.evidence_keys {
                check_key(evidence, member.source.primary)?;
                size.text(evidence);
            }
        }
        for member in input.gate_rules {
            check_key(member.rule_key, member.source.primary)?;
            check_source(member.source, Some(member.rule_key))?;
            self.validate_policy_reference(member.gate, member.source.primary)?;
            if member.participant_classes.is_some_and(<[_]>::is_empty) {
                return Err(fail(
                    Some(member.rule_key),
                    V::EmptyClasses,
                    member.source.primary,
                ));
            }
            size.text(member.rule_key);
            size.qualified(member.gate, &self.header.authoring_namespace_id);
            size.source(member.source);
            for reference in member.participant_classes.unwrap_or(&[]) {
                self.validate_reference(
                    EntityKind::ParticipantClass,
                    *reference,
                    member.source.primary,
                )?;
                size.reference(*reference, &self.header.authoring_namespace_id);
            }
            for evidence in member.evidence_keys {
                check_key(evidence, member.source.primary)?;
                size.text(evidence);
            }
        }
        let members = (input.evidence.len() as u64)
            .saturating_add(input.gap_profiles.len() as u64)
            .saturating_add(input.stream_rules.len() as u64)
            .saturating_add(input.gate_rules.len() as u64);
        size.structural = size
            .structural
            .saturating_add(size_bytes::<RightOfWayPolicySetDeclaration>(1))
            .saturating_add(size_bytes::<PolicyEvidenceDeclaration>(
                input.evidence.len() as u64,
            ))
            .saturating_add(size_bytes::<PolicyGapProfileDeclaration>(
                input.gap_profiles.len() as u64,
            ))
            .saturating_add(size_bytes::<PolicyStreamRuleDeclaration>(
                input.stream_rules.len() as u64,
            ))
            .saturating_add(size_bytes::<PolicyGateRuleDeclaration>(
                input.gate_rules.len() as u64,
            ))
            .saturating_add(size_bytes::<Arc<str>>(size.strings));
        // 编码上界包含每个字符串长度、引用/来源位置、集合长度及最大成员标量形状；
        // 实际 source bytes 在拥有化后重算，只有通过同一预算才提交。
        size.source = size
            .bytes
            .saturating_add(size.strings.saturating_mul(4))
            .saturating_add(size.locations.saturating_mul(16))
            .saturating_add(size.references.saturating_mul(4))
            .saturating_add(members.saturating_mul(48))
            .saturating_add(64);
        let string_limit = self.limits.value(CompileLimitDimension::SingleStringBytes);
        if size.max_string > string_limit {
            return Err(DiagnosticBundle::single(
                Diagnostic::compile_limit_exceeded(
                    CompileLimitDimension::SingleStringBytes,
                    string_limit,
                    size.max_string,
                ),
            ));
        }
        let delta = DeclarationResourceDelta {
            declarations: 1,
            typed_ast_records: 3_u64
                .saturating_add(members)
                .saturating_add(size.references)
                .saturating_add(size.locations),
            references: size.references,
            relations: members
                .saturating_add(size.references)
                .saturating_add(size.locations),
            identity_fields: 2,
            symbols: 1,
            string_items: size.strings,
            string_bytes: size.bytes,
            controlled_string_bytes: size.bytes,
            controlled_structural_bytes: size.structural,
            source_bytes: size.source,
            ..DeclarationResourceDelta::default()
        };
        self.check_declaration_resources(delta, key, span)?;
        let own_source = |value: PolicyInputSource<'_>| PolicyDeclarationSource {
            primary: value.primary.clone().into(),
            contributing: value.contributing.iter().cloned().map(Into::into).collect(),
        };
        let own_keys = |values: &[&str]| -> Box<[Arc<str>]> {
            let mut values: Vec<_> = values
                .iter()
                .map(|value| Arc::<str>::from(*value))
                .collect();
            values.sort_unstable();
            values.into_boxed_slice()
        };
        let mut evidence: Vec<_> = input
            .evidence
            .iter()
            .map(|v| PolicyEvidenceDeclaration {
                key: v.evidence_key.into(),
                locator: v.locator.into(),
                description: v.description.map(Into::into),
                source: own_source(v.source),
            })
            .collect();
        let mut gaps: Vec<_> = input
            .gap_profiles
            .iter()
            .map(|v| PolicyGapProfileDeclaration {
                key: v.profile_key.into(),
                parameter_version: v.parameter_version.into(),
                minimum_lead_gap_ms: v.minimum_lead_gap_ms,
                minimum_lag_gap_ms: v.minimum_lag_gap_ms,
                clearance_buffer_ms: v.clearance_buffer_ms,
                source: own_source(v.source),
            })
            .collect();
        let mut streams = Vec::with_capacity(input.stream_rules.len());
        for v in input.stream_rules {
            let mut yield_to: Box<[_]> = v
                .yield_to_streams
                .iter()
                .map(|r| self.own_policy_reference(*r, v.source.primary))
                .collect::<Result<_, _>>()?;
            crate::policy::sort_references(&mut yield_to);
            streams.push(PolicyStreamRuleDeclaration {
                key: v.rule_key.into(),
                stream: self.own_policy_reference(v.stream, v.source.primary)?,
                classes: v
                    .participant_classes
                    .map(|refs| self.own_policy_classes(refs, v.source.primary))
                    .transpose()?,
                priority: v.priority,
                yield_to,
                gap: v.gap_profile_key.map(Into::into),
                evidence: own_keys(v.evidence_keys),
                source: own_source(v.source),
            });
        }
        let mut gates = Vec::with_capacity(input.gate_rules.len());
        for v in input.gate_rules {
            gates.push(PolicyGateRuleDeclaration {
                key: v.rule_key.into(),
                gate: self.own_policy_reference(v.gate, v.source.primary)?,
                classes: v
                    .participant_classes
                    .map(|refs| self.own_policy_classes(refs, v.source.primary))
                    .transpose()?,
                interpretation: v.interpretation,
                prohibition: v.prohibition,
                evidence: own_keys(v.evidence_keys),
                source: own_source(v.source),
            });
        }
        evidence.sort_unstable_by(|a, b| a.key.cmp(&b.key));
        gaps.sort_unstable_by(|a, b| a.key.cmp(&b.key));
        streams.sort_unstable_by(|a, b| a.key.cmp(&b.key));
        gates.sort_unstable_by(|a, b| a.key.cmp(&b.key));
        let declaration = RightOfWayPolicySetDeclaration {
            header: DeclarationHeader::module_scoped(
                EntityKind::RightOfWayPolicySet,
                key.into(),
                span.clone().into(),
            ),
            regulation: crate::RegulationIdentity {
                jurisdiction: input.regulation.jurisdiction.into(),
                version: input.regulation.version.into(),
                source: input.regulation.source.map(Into::into),
            },
            evidence: evidence.into_boxed_slice(),
            gap_profiles: gaps.into_boxed_slice(),
            stream_rules: streams.into_boxed_slice(),
            gate_rules: gates.into_boxed_slice(),
            contributing: input
                .source
                .contributing
                .iter()
                .cloned()
                .map(Into::into)
                .collect(),
        };
        crate::policy::validate_local_declaration(&declaration)?;
        let declaration = TypedAstDeclaration::RightOfWayPolicySet(declaration);
        let exact_source_bytes =
            crate::module::synthetic_record::encoded_declaration_len(&declaration)
                .expect("official policy encoding");
        debug_assert!(
            exact_source_bytes <= size.source,
            "policy preallocation upper bound"
        );
        let state = self.check_declaration_resources(
            DeclarationResourceDelta {
                source_bytes: exact_source_bytes,
                ..delta
            },
            key,
            span,
        )?;
        let TypedAstDeclaration::RightOfWayPolicySet(source) = &declaration else {
            unreachable!()
        };
        self.declaration_index
            .entry(EntityKind::RightOfWayPolicySet)
            .or_default()
            .insert(
                Arc::clone(&source.header.stable_key),
                source.header.span.clone(),
            );
        self.declarations.push(declaration);
        self.commit_declaration_resources(state);
        Ok(self)
    }

    fn validate_policy_reference<K: EntityKindMarker>(
        &self,
        reference: OwnerQualifiedReference<'_, K>,
        span: &SourceSpan,
    ) -> Result<(), DiagnosticBundle> {
        self.validate_reference(K::KIND, reference.target, span)?;
        if reference.owner_keys.len() > 3 {
            return Err(crate::policy::error(
                reference.target.declaration_key(),
                None,
                V::InvalidKey,
                &span.clone().into(),
            ));
        }
        for key in reference.owner_keys {
            if let Some(violation) =
                external_token_violation(key, self.limits.identity_ascii_bytes_limit())
            {
                return Err(DiagnosticBundle::single(Diagnostic::invalid_reference_key(
                    K::KIND,
                    violation,
                    span.clone(),
                )));
            }
        }
        Ok(())
    }
    fn own_policy_reference<K: EntityKindMarker>(
        &self,
        reference: OwnerQualifiedReference<'_, K>,
        span: &SourceSpan,
    ) -> Result<OwnedEntityReference<K>, DiagnosticBundle> {
        let key = Arc::<str>::from(reference.target.declaration_key());
        let address = if reference.owner_keys.is_empty() {
            TypedAstEntityAddress::module_scoped(key)
        } else {
            TypedAstEntityAddress::owner_scoped(
                reference
                    .owner_keys
                    .iter()
                    .map(|key| Arc::<str>::from(*key))
                    .collect(),
                key,
            )
        };
        Ok(OwnedEntityReference::with_target_address(
            self.reference_namespace_arc(reference.target.module_namespace(), span)?,
            address,
            span.clone(),
        ))
    }
    fn own_policy_classes(
        &self,
        references: &[ParticipantClassReference<'_>],
        span: &SourceSpan,
    ) -> Result<Box<[OwnedEntityReference<ParticipantClassKind>]>, DiagnosticBundle> {
        let mut values: Box<[_]> = references
            .iter()
            .map(|r| self.own_reference(EntityKind::ParticipantClass, *r, span))
            .collect::<Result<_, _>>()?;
        crate::policy::sort_references(&mut values);
        Ok(values)
    }
}
