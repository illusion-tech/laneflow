//! 策略来源记录共享计量/编码遍历；不分配第二份序列化缓存。
use crate::SourceLocation;
use crate::declaration::{
    OwnedEntityReference, PolicyDeclarationSource, RightOfWayPolicySetDeclaration,
};
use laneflow_static_contract::EntityKindMarker;

trait Sink {
    fn bytes(&mut self, value: &[u8]);
    fn count(&mut self, count: usize) {
        self.bytes(&u32::try_from(count).unwrap_or(u32::MAX).to_le_bytes());
    }
    fn text(&mut self, value: &str) {
        self.count(value.len());
        self.bytes(value.as_bytes());
    }
    fn optional(&mut self, value: Option<&str>) {
        self.bytes(&[u8::from(value.is_some())]);
        if let Some(v) = value {
            self.text(v);
        }
    }
    fn location(&mut self, value: &SourceLocation) {
        let span = value
            .text_span()
            .expect("Synthetic has checked Text sources");
        for value in [
            span.start().line(),
            span.start().column(),
            span.end().line(),
            span.end().column(),
        ] {
            self.bytes(&value.to_le_bytes());
        }
    }
    fn source(&mut self, value: &PolicyDeclarationSource) {
        self.location(&value.primary);
        self.count(value.contributing.len());
        for location in &value.contributing {
            self.location(location);
        }
    }
    fn reference<K: EntityKindMarker>(&mut self, value: &OwnedEntityReference<K>) {
        self.text(&value.module_namespace);
        self.count(value.target_address.owner_local_keys().len());
        for owner in value.target_address.owner_local_keys() {
            self.text(owner);
        }
        self.text(value.declaration_key());
        self.location(&value.span);
    }
    fn references<K: EntityKindMarker>(&mut self, values: &[OwnedEntityReference<K>]) {
        self.count(values.len());
        for value in values {
            self.reference(value);
        }
    }
    fn classes(
        &mut self,
        values: &Option<
            Box<[OwnedEntityReference<laneflow_static_contract::ParticipantClassKind>]>,
        >,
    ) {
        self.bytes(&[u8::from(values.is_some())]);
        if let Some(values) = values {
            self.references(values);
        }
    }
    fn evidence(&mut self, values: &[std::sync::Arc<str>]) {
        self.count(values.len());
        for value in values {
            self.text(value);
        }
    }
}
impl Sink for Vec<u8> {
    fn bytes(&mut self, value: &[u8]) {
        self.extend_from_slice(value);
    }
}
struct Count(u64);
impl Sink for Count {
    fn bytes(&mut self, value: &[u8]) {
        self.0 = self.0.saturating_add(value.len() as u64);
    }
}

fn write(value: &RightOfWayPolicySetDeclaration, output: &mut impl Sink) {
    output.bytes(&24_u16.to_le_bytes());
    output.text(&value.header.stable_key);
    output.location(&value.header.span);
    output.text(&value.regulation.jurisdiction);
    output.text(&value.regulation.version);
    output.optional(value.regulation.source.as_deref());
    output.count(value.contributing.len());
    for location in &value.contributing {
        output.location(location);
    }
    output.count(value.evidence.len());
    for v in &value.evidence {
        output.text(&v.key);
        output.text(&v.locator);
        output.optional(v.description.as_deref());
        output.source(&v.source);
    }
    output.count(value.gap_profiles.len());
    for v in &value.gap_profiles {
        output.text(&v.key);
        output.text(&v.parameter_version);
        for number in [
            v.minimum_lead_gap_ms,
            v.minimum_lag_gap_ms,
            v.clearance_buffer_ms,
        ] {
            output.bytes(&number.to_le_bytes());
        }
        output.source(&v.source);
    }
    output.count(value.stream_rules.len());
    for v in &value.stream_rules {
        output.text(&v.key);
        output.reference(&v.stream);
        output.classes(&v.classes);
        output.bytes(&v.priority.to_le_bytes());
        output.references(&v.yield_to);
        output.optional(v.gap.as_deref());
        output.evidence(&v.evidence);
        output.source(&v.source);
    }
    output.count(value.gate_rules.len());
    for v in &value.gate_rules {
        output.text(&v.key);
        output.reference(&v.gate);
        output.classes(&v.classes);
        output.bytes(&[v.interpretation.code(), v.prohibition.code()]);
        output.evidence(&v.evidence);
        output.source(&v.source);
    }
}
pub(super) fn length(value: &RightOfWayPolicySetDeclaration) -> u64 {
    let mut count = Count(0);
    write(value, &mut count);
    count.0
}
pub(super) fn encode(value: &RightOfWayPolicySetDeclaration, output: &mut Vec<u8>) {
    write(value, output);
}
