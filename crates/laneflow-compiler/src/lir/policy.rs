//! 策略声明冻结；实际执行行仅在语义校验期间暂存，不进入 LFCA。
use super::*;
use crate::policy::model::PolicySet;
use laneflow_static_contract::RightOfWayPolicySetOrdinal;

pub(crate) struct LirPolicy {
    pub ordinal: RightOfWayPolicySetOrdinal,
    pub identity_fields: TableRange<LirIdentityField>,
    pub value: PolicySet<ManeuverGateOrdinal, ParticipantStreamOrdinal, ParticipantClassOrdinal>,
}

pub(super) fn freeze(env: &mut FreezeEnv<'_>) -> Result<Box<[LirPolicy]>, DiagnosticBundle> {
    let mut policies = Vec::with_capacity(env.mir.policies.len());
    // HIR 已按完整 Identity 前像排序，且没有策略间引用，不再建立第二张排列索引。
    for (index, record) in env.mir.policies.iter().enumerate() {
        let identity_fields = push_lir_identity(
            env.identity_fields,
            env.identity_field_bytes,
            FieldTag::RightOfWayPolicySetKey,
            &record.value.namespace,
            &record.value.key,
            None,
            env.limits,
            env.primary_span.clone(),
        )?;
        let mut value = record.value.map(
            |g| env.orders.maneuver_gates.ordinal(g),
            |s| env.orders.participant_streams.ordinal(s),
            |c| env.orders.participant_classes.ordinal(c),
        );
        let class_id = |c: &ParticipantClassOrdinal| {
            let key = env
                .orders
                .participant_classes
                .stage_key_at_lir_index(c.raw() as usize);
            env.mir.participant_classes[key.index()].stable_id
        };
        for rule in &mut value.streams {
            if let Some(classes) = &mut rule.classes {
                classes.sort_unstable_by_key(class_id);
            }
            rule.yield_to.sort_unstable_by_key(|s| {
                let key = env
                    .orders
                    .participant_streams
                    .stage_key_at_lir_index(s.raw() as usize);
                env.mir.participant_streams[key.index()].stable_id
            });
        }
        for rule in &mut value.gates {
            if let Some(classes) = &mut rule.classes {
                classes.sort_unstable_by_key(class_id);
            }
        }
        policies.push(LirPolicy {
            ordinal: RightOfWayPolicySetOrdinal::from_raw(index as u32),
            identity_fields,
            value,
        });
    }
    Ok(policies.into_boxed_slice())
}

/// 输出独立存续时连同共享字符串 payload 一并计量；逐项保守计，避免隐含缓存。
pub(super) fn bytes(mir: &MirUnit) -> u64 {
    mir.policies.iter().fold(0_u64, |n, record| {
        n.saturating_add(record.value.owned_bytes())
            .saturating_add(16)
            .saturating_add(record.value.text_bytes())
    })
}

pub(super) fn hash(hasher: &mut blake3::Hasher, policies: &[LirPolicy]) {
    hash_u32(hasher, EntityKind::RightOfWayPolicySet.code().into());
    hash_u32(hasher, policies.len() as u32);
    fn text(h: &mut blake3::Hasher, v: Option<&str>) {
        h.update(&[u8::from(v.is_some())]);
        if let Some(v) = v {
            hash_bytes(h, v.as_bytes());
        }
    }
    fn classes(h: &mut blake3::Hasher, v: &Option<Box<[ParticipantClassOrdinal]>>) {
        h.update(&[u8::from(v.is_some())]);
        if let Some(v) = v {
            hash_u32(h, v.len() as u32);
            for c in v {
                hash_u32(h, c.raw());
            }
        }
    }
    for record in policies {
        let p = &record.value;
        hash_u32(hasher, record.ordinal.raw());
        hash_bytes(hasher, p.namespace.as_bytes());
        hash_bytes(hasher, p.key.as_bytes());
        hash_bytes(hasher, p.regulation.jurisdiction.as_bytes());
        hash_bytes(hasher, p.regulation.version.as_bytes());
        text(hasher, p.regulation.source.as_deref());
        hash_u32(hasher, p.evidence.len() as u32);
        for e in &p.evidence {
            hash_bytes(hasher, e.key.as_bytes());
            hash_bytes(hasher, e.locator.as_bytes());
            text(hasher, e.description.as_deref());
        }
        hash_u32(hasher, p.gaps.len() as u32);
        for g in &p.gaps {
            hash_bytes(hasher, g.key.as_bytes());
            hash_bytes(hasher, g.parameter_version.as_bytes());
            for v in [g.lead_ms, g.lag_ms, g.clearance_ms] {
                hasher.update(&v.to_le_bytes());
            }
        }
        hash_u32(hasher, p.streams.len() as u32);
        for r in &p.streams {
            hash_bytes(hasher, r.key.as_bytes());
            hash_u32(hasher, r.stream.raw());
            classes(hasher, &r.classes);
            hasher.update(&r.priority.to_le_bytes());
            hash_u32(hasher, r.yield_to.len() as u32);
            for s in &r.yield_to {
                hash_u32(hasher, s.raw());
            }
            text(hasher, r.gap.as_deref());
            hash_u32(hasher, r.evidence.len() as u32);
            for e in &r.evidence {
                hash_bytes(hasher, e.as_bytes());
            }
        }
        hash_u32(hasher, p.gates.len() as u32);
        for r in &p.gates {
            hash_bytes(hasher, r.key.as_bytes());
            hash_u32(hasher, r.gate.raw());
            classes(hasher, &r.classes);
            hasher.update(&[r.interpretation.code(), r.prohibition.code()]);
            hash_u32(hasher, r.evidence.len() as u32);
            for e in &r.evidence {
                hash_bytes(hasher, e.as_bytes());
            }
        }
    }
}
