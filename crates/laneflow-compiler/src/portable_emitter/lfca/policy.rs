use super::*;
use crate::lir::{LirPolicy, LirUnit};
use std::sync::Arc;

pub(super) fn identities(lir: &LirUnit) -> impl Iterator<Item = OwnedRow> + '_ {
    lir.policies.iter().map(|p| {
        row([
            field(1, OwnedValue::U16(EntityKind::RightOfWayPolicySet.code())),
            field(2, OwnedValue::U32(p.ordinal.raw())),
            field(3, OwnedValue::StableId128(stable_id_bytes(p.value.id))),
            field(4, identity_fields(lir, p.identity_fields)),
        ])
    })
}
pub(super) fn declarations(lir: &LirUnit) -> impl Iterator<Item = OwnedRow> + '_ {
    lir.policies.iter().map(|p| {
        let mut fields = vec![
            field(1, OwnedValue::U32(p.ordinal.raw())),
            field(2, OwnedValue::StableId128(stable_id_bytes(p.value.id))),
            field(
                3,
                OwnedValue::Utf8(p.value.regulation.jurisdiction.to_string().into_boxed_str()),
            ),
            field(
                4,
                OwnedValue::Utf8(p.value.regulation.version.to_string().into_boxed_str()),
            ),
        ];
        if let Some(s) = &p.value.regulation.source {
            fields.push(field(5, OwnedValue::Utf8(s.to_string().into_boxed_str())));
        }
        row(fields)
    })
}
fn key_fields(p: &LirPolicy, key: &str) -> Vec<OwnedField> {
    vec![
        field(1, OwnedValue::U32(p.ordinal.raw())),
        field(2, OwnedValue::Utf8(key.into())),
    ]
}
fn text(v: &str) -> OwnedValue {
    OwnedValue::Utf8(v.into())
}
fn evidence(v: &[Arc<str>]) -> OwnedValue {
    OwnedValue::RecordVector(v.iter().map(|s| row([field(1, text(s))])).collect())
}

pub(super) fn evidence_rows(lir: &LirUnit) -> impl Iterator<Item = OwnedRow> + '_ {
    lir.policies.iter().flat_map(|p| {
        p.value.evidence.iter().map(move |v| {
            let mut fields = key_fields(p, &v.key);
            fields.push(field(3, text(&v.locator)));
            if let Some(d) = &v.description {
                fields.push(field(4, text(d)));
            }
            row(fields)
        })
    })
}
pub(super) fn gap_rows(lir: &LirUnit) -> impl Iterator<Item = OwnedRow> + '_ {
    lir.policies.iter().flat_map(|p| {
        p.value.gaps.iter().map(move |v| {
            let mut fields = key_fields(p, &v.key);
            fields.extend([
                field(3, text(&v.parameter_version)),
                field(4, OwnedValue::U64(v.lead_ms)),
                field(5, OwnedValue::U64(v.lag_ms)),
                field(6, OwnedValue::U64(v.clearance_ms)),
            ]);
            row(fields)
        })
    })
}
pub(super) fn stream_rows(lir: &LirUnit) -> impl Iterator<Item = OwnedRow> + '_ {
    lir.policies.iter().flat_map(|p| {
        p.value.streams.iter().map(move |v| {
            let mut fields = key_fields(p, &v.key);
            fields.push(field(3, OwnedValue::U32(v.stream.raw())));
            if let Some(classes) = &v.classes {
                fields.push(field(4, OwnedValue::OrdinalVectorU32(ordinals(classes))));
            }
            fields.extend([
                field(5, OwnedValue::I32(v.priority)),
                field(6, OwnedValue::OrdinalVectorU32(ordinals(&v.yield_to))),
            ]);
            if let Some(gap) = &v.gap {
                fields.push(field(7, text(gap)));
            }
            fields.push(field(8, evidence(&v.evidence)));
            row(fields)
        })
    })
}
pub(super) fn gate_rows(lir: &LirUnit) -> impl Iterator<Item = OwnedRow> + '_ {
    lir.policies.iter().flat_map(|p| {
        p.value.gates.iter().map(move |v| {
            let mut fields = key_fields(p, &v.key);
            fields.push(field(3, OwnedValue::U32(v.gate.raw())));
            if let Some(classes) = &v.classes {
                fields.push(field(4, OwnedValue::OrdinalVectorU32(ordinals(classes))));
            }
            fields.extend([
                field(5, OwnedValue::U8(v.interpretation.code())),
                field(6, OwnedValue::U8(v.prohibition.code())),
                field(7, evidence(&v.evidence)),
            ]);
            row(fields)
        })
    })
}
