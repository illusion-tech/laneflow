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
        usage.relation_occurrence_count = usage
            .relation_occurrence_count
            .saturating_add(6 + u64::from(v.classes.is_some()) + v.evidence.len() as u64);
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
