use super::super::tree::{TableRef, TreeBuilder, Value, field};
use super::*;
use crate::lir::{LirPolicy, LirUnit};
use std::sync::Arc;

/// 为每份 LIR 路权策略集写入 LFCA 规范身份行。
pub(super) fn identities(t: &TreeBuilder, lir: &LirUnit) {
    for p in &lir.policies {
        t.row([
            field(1, Value::U16(EntityKind::RightOfWayPolicySet.code())),
            field(2, Value::U32(p.ordinal.raw())),
            field(3, Value::StableId128(stable_id_bytes(p.value.id))),
            field(4, identity_fields(t, lir, p.identity_fields)),
        ]);
    }
}

/// 写入 LFCA 路权策略集声明表（实体节第 24 张表），含法规管辖、版本与可选来源。
pub(super) fn declarations(t: &TreeBuilder, lir: &LirUnit) -> TableRef {
    t.table(
        24,
        lir.policies.iter().map(|p| {
            t.row([
                Some(field(1, Value::U32(p.ordinal.raw()))),
                Some(field(2, Value::StableId128(stable_id_bytes(p.value.id)))),
                Some(field(3, t.utf8(&p.value.regulation.jurisdiction))),
                Some(field(4, t.utf8(&p.value.regulation.version))),
                p.value
                    .regulation
                    .source
                    .as_ref()
                    .map(|s| field(5, t.utf8(s))),
            ])
        }),
    )
}

fn evidence(t: &TreeBuilder, v: &[Arc<str>]) -> Value {
    t.records(v.iter().map(|s| t.record([field(1, t.utf8(s))])))
}

/// 每份策略集的依据（evidence）逐条展开为 LFCA 关系表，含 key、locator 与可选描述。
pub(super) fn evidence_table(t: &TreeBuilder, lir: &LirUnit) -> TableRef {
    t.table(
        2,
        lir.policies.iter().flat_map(|p| {
            p.value.evidence.iter().map(move |v| {
                t.row([
                    Some(field(1, Value::U32(p.ordinal.raw()))),
                    Some(field(2, t.utf8(&v.key))),
                    Some(field(3, t.utf8(&v.locator))),
                    v.description.as_ref().map(|d| field(4, t.utf8(d))),
                ])
            })
        }),
    )
}

/// 每份策略集的间隙接受参数逐条展开为 LFCA 关系表，含参数版本与领先/滞后/清空毫秒。
pub(super) fn gap_table(t: &TreeBuilder, lir: &LirUnit) -> TableRef {
    t.table(
        3,
        lir.policies.iter().flat_map(|p| {
            p.value.gaps.iter().map(move |v| {
                t.row([
                    field(1, Value::U32(p.ordinal.raw())),
                    field(2, t.utf8(&v.key)),
                    field(3, t.utf8(&v.parameter_version)),
                    field(4, Value::U64(v.lead_ms)),
                    field(5, Value::U64(v.lag_ms)),
                    field(6, Value::U64(v.clearance_ms)),
                ])
            })
        }),
    )
}

fn key_field(t: &TreeBuilder, p: &LirPolicy, key: &str) -> [Option<super::super::tree::Field>; 2] {
    [
        Some(field(1, Value::U32(p.ordinal.raw()))),
        Some(field(2, t.utf8(key))),
    ]
}

/// 每份策略集的通行流路权规则逐条展开为 LFCA 关系表，含目标参与者流、可选参与者
/// 类别、优先级、让行集合、可选间隙参数引用与依据。
pub(super) fn stream_table(t: &TreeBuilder, lir: &LirUnit) -> TableRef {
    t.table(
        4,
        lir.policies.iter().flat_map(|p| {
            p.value.streams.iter().map(move |v| {
                let [ordinal, key] = key_field(t, p, &v.key);
                t.row([
                    ordinal,
                    key,
                    Some(field(3, Value::U32(v.stream.raw()))),
                    v.classes
                        .as_ref()
                        .map(|classes| field(4, t.ordinals_of(classes))),
                    Some(field(5, Value::I32(v.priority))),
                    Some(field(6, t.ordinals_of(&v.yield_to))),
                    v.gap.as_ref().map(|gap| field(7, t.utf8(gap))),
                    Some(field(8, evidence(t, &v.evidence))),
                ])
            })
        }),
    )
}

/// 每份策略集的门合规规则逐条展开为 LFCA 关系表，含目标机动门、可选参与者类别、
/// 解释与禁止代码及依据。
pub(super) fn gate_table(t: &TreeBuilder, lir: &LirUnit) -> TableRef {
    t.table(
        5,
        lir.policies.iter().flat_map(|p| {
            p.value.gates.iter().map(move |v| {
                let [ordinal, key] = key_field(t, p, &v.key);
                t.row([
                    ordinal,
                    key,
                    Some(field(3, Value::U32(v.gate.raw()))),
                    v.classes
                        .as_ref()
                        .map(|classes| field(4, t.ordinals_of(classes))),
                    Some(field(5, Value::U8(v.interpretation.code()))),
                    Some(field(6, Value::U8(v.prohibition.code()))),
                    Some(field(7, evidence(t, &v.evidence))),
                ])
            })
        }),
    )
}
