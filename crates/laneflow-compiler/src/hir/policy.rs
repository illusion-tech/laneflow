//! 在既有静态语义诊断之后闭合所有策略及跨模块引用。
use super::*;
use crate::declaration::{OwnedEntityReference, RightOfWayPolicySetDeclaration};
use crate::identity::{IdentityFieldInput, IdentityRegistry};
use crate::policy::model::*;
use laneflow_static_contract::{EntityKind, EntityKindMarker, FieldTag, RightOfWayPolicySetId};
use std::sync::Arc;

/// 绑定到 HIR 阶段致密键的通行权策略集记录。
pub(crate) type HirPolicy =
    PolicyRecord<HirManeuverGateKey, HirParticipantStreamKey, HirParticipantClassKey>;
struct Lookup<'a, K> {
    namespace: &'a str,
    owners: &'a [Arc<str>],
    key: &'a str,
    value: K,
}
fn order<K>(values: &mut [Lookup<'_, K>]) {
    values.sort_unstable_by(|a, b| {
        a.namespace
            .cmp(b.namespace)
            .then(a.owners.cmp(b.owners))
            .then(a.key.cmp(b.key))
    });
}
fn resolve<K: Copy, M: EntityKindMarker>(
    values: &[Lookup<'_, K>],
    reference: &OwnedEntityReference<M>,
    source: &RightOfWayPolicySetDeclaration,
) -> Result<K, DiagnosticBundle> {
    let index = values.binary_search_by(|v| {
        v.namespace
            .cmp(&reference.module_namespace)
            .then(v.owners.cmp(reference.target_address.owner_local_keys()))
            .then(v.key.cmp(reference.declaration_key()))
    });
    match index {
        Ok(index) => Ok(values[index].value),
        Err(_) => Err(DiagnosticBundle::single(
            Diagnostic::unknown_owner_qualified_reference_target(
                EntityKind::RightOfWayPolicySet,
                &source.header.stable_key,
                &reference.module_namespace,
                reference.target_address.owner_local_keys(),
                reference.declaration_key(),
                reference.span.clone(),
                source.header.span.clone(),
            ),
        )),
    }
}
fn declaration_size(v: &RightOfWayPolicySetDeclaration) -> (u64, u64) {
    let mut bytes = size_of::<HirPolicy>() as u64;
    bytes = bytes
        .saturating_add((v.evidence.len() as u64).saturating_mul(size_of::<Evidence>() as u64))
        .saturating_add((v.gap_profiles.len() as u64).saturating_mul(size_of::<Gap>() as u64))
        .saturating_add((v.stream_rules.len() as u64).saturating_mul(size_of::<
            StreamRule<HirParticipantStreamKey, HirParticipantClassKey>,
        >() as u64))
        .saturating_add((v.gate_rules.len() as u64).saturating_mul(size_of::<
            GateRule<HirManeuverGateKey, HirParticipantClassKey>,
        >() as u64));
    let mut records = 1_u64
        .saturating_add(v.evidence.len() as u64)
        .saturating_add(v.gap_profiles.len() as u64)
        .saturating_add(v.stream_rules.len() as u64)
        .saturating_add(v.gate_rules.len() as u64);
    for r in &v.stream_rules {
        let refs = r.classes.as_ref().map_or(0, |v| v.len()) as u64 + r.yield_to.len() as u64;
        bytes = bytes
            .saturating_add(refs.saturating_mul(4))
            .saturating_add((r.evidence.len() as u64).saturating_mul(size_of::<Arc<str>>() as u64));
        records = records
            .saturating_add(refs)
            .saturating_add(r.evidence.len() as u64);
    }
    for r in &v.gate_rules {
        let refs = r.classes.as_ref().map_or(0, |v| v.len()) as u64;
        bytes = bytes
            .saturating_add(refs.saturating_mul(4))
            .saturating_add((r.evidence.len() as u64).saturating_mul(size_of::<Arc<str>>() as u64));
        records = records
            .saturating_add(refs)
            .saturating_add(r.evidence.len() as u64);
    }
    (bytes, records)
}

/// 在既有静态语义诊断之后，把全部通行权策略集声明绑定进 `HirUnit` 并闭合其跨模块引用。
/// 策略绑定的计数与计划：声明数、策略存储字节、记录数与子编译遍计划峰值。
#[derive(Clone, Copy)]
pub(super) struct PolicyBindPlan {
    count: usize,
    owned: u64,
    records: u64,
    pub(super) peak: crate::ledger::Plan,
}

impl PolicyBindPlan {
    /// 按策略声明与 HIR 机动门、参与者流、参与者类别的数量求出计划；不分配内存。
    ///
    /// 策略存储（记录表与记录内嵌的规则、证据与引用）随 HIR 存续，三张查找表只在绑定
    /// 期间存续。
    /// 无策略声明时的空计划。
    pub(super) const EMPTY: Self = Self {
        count: 0,
        owned: 0,
        records: 0,
        peak: crate::ledger::Plan::ZERO,
    };

    pub(super) fn analyze(
        unit: &CompilationUnit,
        gates: usize,
        streams: usize,
        classes: usize,
    ) -> Self {
        let mut count = 0_usize;
        let mut owned = 0_u64;
        let mut records = 0_u64;
        for module in &unit.modules {
            for declaration in &module.declarations {
                if let TypedAstDeclaration::RightOfWayPolicySet(v) = declaration {
                    count = count.saturating_add(1);
                    let (bytes, rows) = declaration_size(v);
                    owned = owned.saturating_add(bytes);
                    records = records.saturating_add(rows);
                }
            }
        }
        let peak = if count == 0 {
            crate::ledger::Plan::ZERO
        } else {
            crate::ledger::Plan::of(owned)
                .array::<Lookup<'static, HirManeuverGateKey>>(gates)
                .array::<Lookup<'static, HirParticipantStreamKey>>(streams)
                .array::<Lookup<'static, HirParticipantClassKey>>(classes)
        };
        Self {
            count,
            owned,
            records,
            peak,
        }
    }

    /// 绑定结束后随 HIR 存续的策略存储字节。
    pub(super) const fn output_bytes(&self) -> u64 {
        if self.count == 0 { 0 } else { self.owned }
    }
}

/// 在 HIR 编译遍内绑定策略：从 HIR 编译遍划出子编译遍，查找表与策略存储都经其授予创建。
pub(super) fn bind(
    unit: &CompilationUnit,
    parent: &crate::ledger::PassBudget<'_>,
    plan: &PolicyBindPlan,
    hir: &mut HirUnit,
    identities: &mut IdentityRegistry,
) -> Result<(), DiagnosticBundle> {
    let PolicyBindPlan {
        count,
        owned,
        records,
        peak,
    } = *plan;
    if count == 0 {
        return Ok(());
    }
    let pass = parent.admit_sub(POLICY_BIND_PASS, peak)?;
    let total_records = hir.hir_record_count.saturating_add(records);
    let limit = unit
        .limits
        .source_limits()
        .get(crate::SourceProcessingDimension::HirRecordCount);
    if total_records > limit {
        return Err(DiagnosticBundle::single(
            Diagnostic::compile_limit_exceeded(
                CompileLimitDimension::HirRecordCount,
                limit,
                total_records,
            ),
        ));
    }
    let mut gates = pass.collect(hir.maneuver_gates.iter().enumerate().map(|(i, v)| Lookup {
        namespace: &hir.modules[v.module.index()].authoring_namespace_id,
        owners: v.source_address.owner_local_keys(),
        key: &v.stable_key,
        value: HirManeuverGateKey::from_raw(i as u32),
    }))?;
    let mut streams =
        pass.collect(
            hir.participant_streams
                .iter()
                .enumerate()
                .map(|(i, v)| Lookup {
                    namespace: &hir.modules[v.module.index()].authoring_namespace_id,
                    owners: v.source_address.owner_local_keys(),
                    key: &v.stable_key,
                    value: HirParticipantStreamKey::from_raw(i as u32),
                }),
        )?;
    let mut classes =
        pass.collect(
            hir.participant_classes
                .iter()
                .enumerate()
                .map(|(i, v)| Lookup {
                    namespace: &hir.modules[v.module.index()].authoring_namespace_id,
                    owners: &[],
                    key: &v.stable_key,
                    value: HirParticipantClassKey::from_raw(i as u32),
                }),
        )?;
    order(&mut gates);
    order(&mut streams);
    order(&mut classes);
    let mut policies = pass.vec(count)?;
    // 记录内嵌的规则、证据与引用集中取得一笔授予；记录表本身逐表授予。
    let payload_bytes = owned.saturating_sub(policies.granted_bytes());
    let payloads = pass.grant_bytes(payload_bytes)?;
    for (module_index, module) in unit.modules.iter().enumerate() {
        for (declaration_index, declaration) in module.declarations.iter().enumerate() {
            let TypedAstDeclaration::RightOfWayPolicySet(source) = declaration else {
                continue;
            };
            crate::policy::validate_local_declaration(source)?;
            let namespace = &hir.modules[module_index].authoring_namespace_id;
            let id = derive_identity(
                unit,
                identities,
                module_index,
                EntityKind::RightOfWayPolicySet,
                &source.header.stable_key,
                &source.header.span,
                &[
                    IdentityFieldInput::new(FieldTag::AuthoringNamespaceId, namespace.as_bytes()),
                    IdentityFieldInput::new(
                        FieldTag::RightOfWayPolicySetKey,
                        source.header.stable_key.as_bytes(),
                    ),
                ],
            )?;
            let resolve_classes = |values: &Option<
                Box<[OwnedEntityReference<laneflow_static_contract::ParticipantClassKind>]>,
            >|
             -> Result<
                Option<Box<[HirParticipantClassKey]>>,
                DiagnosticBundle,
            > {
                values
                    .as_ref()
                    .map(|v| v.iter().map(|v| resolve(&classes, v, source)).collect())
                    .transpose()
            };
            let stream_rules = source
                .stream_rules
                .iter()
                .map(|v| {
                    Ok(StreamRule {
                        key: Arc::clone(&v.key),
                        stream: resolve(&streams, &v.stream, source)?,
                        classes: resolve_classes(&v.classes)?,
                        priority: v.priority,
                        yield_to: v
                            .yield_to
                            .iter()
                            .map(|v| resolve(&streams, v, source))
                            .collect::<Result<_, _>>()?,
                        gap: v.gap.clone(),
                        evidence: v.evidence.clone(),
                    })
                })
                .collect::<Result<Box<[_]>, DiagnosticBundle>>()?;
            let gate_rules = source
                .gate_rules
                .iter()
                .map(|v| {
                    Ok(GateRule {
                        key: Arc::clone(&v.key),
                        gate: resolve(&gates, &v.gate, source)?,
                        classes: resolve_classes(&v.classes)?,
                        interpretation: v.interpretation,
                        prohibition: v.prohibition,
                        evidence: v.evidence.clone(),
                    })
                })
                .collect::<Result<Box<[_]>, DiagnosticBundle>>()?;
            policies.push(PolicyRecord {
                origin: PolicyOrigin {
                    module: module_index as u32,
                    declaration: declaration_index as u32,
                },
                value: PolicySet {
                    id: RightOfWayPolicySetId::from_untyped(id),
                    namespace: Arc::clone(namespace),
                    key: Arc::clone(&source.header.stable_key),
                    regulation: source.regulation.clone(),
                    evidence: source
                        .evidence
                        .iter()
                        .map(|v| Evidence {
                            key: Arc::clone(&v.key),
                            locator: Arc::clone(&v.locator),
                            description: v.description.clone(),
                        })
                        .collect(),
                    gaps: source
                        .gap_profiles
                        .iter()
                        .map(|v| Gap {
                            key: Arc::clone(&v.key),
                            parameter_version: Arc::clone(&v.parameter_version),
                            lead_ms: v.minimum_lead_gap_ms,
                            lag_ms: v.minimum_lag_gap_ms,
                            clearance_ms: v.clearance_buffer_ms,
                        })
                        .collect(),
                    streams: stream_rules,
                    gates: gate_rules,
                },
            })?;
        }
    }
    policies.sort_unstable_by(|a, b| {
        crate::policy::compare_identity_text(&a.value.namespace, &b.value.namespace)
            .then_with(|| crate::policy::compare_identity_text(&a.value.key, &b.value.key))
    });
    drop((gates, streams, classes));
    hir.policies = policies.freeze()?;
    hir.policy_payloads = crate::ledger::OutputGrant::new(payloads);
    hir.hir_record_count = total_records;
    hir.controlled_live_bytes = hir.controlled_live_bytes.saturating_add(owned);
    // 查找表已释放；策略存储转为 HIR 编译遍的输出存续。
    pass.finish(owned)?;
    Ok(())
}

const POLICY_BIND_PASS: crate::ledger::PassLabel =
    crate::ledger::PassLabel::new(crate::ledger::LedgerStage::CompilePipeline, "hir-policy");
