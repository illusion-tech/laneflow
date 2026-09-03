//! emitter 从受检来源按 owner/kind/key 独立排序；不从实际 LFSM 行反推成员。

use super::super::lfsd::policy_change::{Scratch, reserved};
use super::*;
use crate::PolicySourceTarget;

pub(super) fn append_policy_sources(
    source_map: &crate::ValidatedSourceMapInput,
    documents: &DocumentOrdinals<'_>,
    stable: &mut Vec<StableSourceProjection>,
    members: &mut Vec<OwnerLocalProjection>,
    limits: &crate::CompileLimits,
) -> Result<(), PortableEmissionError> {
    let mut scratch = Scratch::new(limits.value(CompileLimitDimension::StageScratchBytes));
    let mut sources =
        reserved::<crate::PolicySourceView<'_>>(source_map.policy_sources().len(), &mut scratch)?;
    sources.extend(source_map.policy_sources());
    sources.sort_unstable_by(|a, b| a.target().cmp(b.target()));
    if sources.windows(2).any(|w| w[0].target() == w[1].target()) {
        return Err(PortableEmissionError::PolicySourceMismatch);
    }
    // 新字段投影、位置池/行编码同时存续的峰值：按每个真实位置最坏复制次数先收费。
    // 嵌套路径逐项计量，字符串长度来自受检原值，不能只数外层来源行。
    let mut stable_count = 0;
    let mut member_count = 0;
    for source in &sources {
        match source.target() {
            PolicySourceTarget::Declaration { .. } => stable_count += 1,
            PolicySourceTarget::Member { .. } => member_count += 1,
            PolicySourceTarget::MovementDirection { .. } => {}
        }
        for location in
            core::iter::once(source.primary_source()).chain(source.contributing_sources())
        {
            let heap = match location {
                crate::SourceLocationView::Text { .. } => 0,
                crate::SourceLocationView::RoadEditing(l) => {
                    let strings = match l.subject() {
                        crate::RoadEditingSubject::Declaration { address }
                        | crate::RoadEditingSubject::RoadAlignment { address } => {
                            address.module_namespace(l.context()).len()
                                + address.local_key(l.context()).len()
                                + address
                                    .owner_local_keys(l.context())
                                    .map(str::len)
                                    .sum::<usize>()
                        }
                        crate::RoadEditingSubject::OwnerLocal {
                            owner: crate::RoadEditingOwner::Address(a),
                            ..
                        } => {
                            a.module_namespace(l.context()).len()
                                + a.local_key(l.context()).len()
                                + a.owner_local_keys(l.context()).map(str::len).sum::<usize>()
                        }
                        _ => 0,
                    };
                    (strings
                        + l.canvas_selection().map_or(0, str::len)
                        + l.property_path().map_or(0, |p| {
                            p.steps().len()
                                * (size_of::<(u8, u16, u16)>()
                                    + size_of::<OwnedRow>()
                                    + 3 * size_of::<OwnedField>())
                        })) as u64
                }
            };
            scratch.charge(
                4 * (heap
                    + size_of::<LocationValue>() as u64
                    + 21 * size_of::<OwnedField>() as u64),
            )?;
        }
    }
    scratch.charge(
        (stable_count * size_of::<StableSourceProjection>()
            + member_count * size_of::<OwnerLocalProjection>()) as u64,
    )?;
    stable
        .try_reserve_exact(stable_count)
        .map_err(|_| PortableEmissionError::AllocationFailure)?;
    members
        .try_reserve_exact(member_count)
        .map_err(|_| PortableEmissionError::AllocationFailure)?;
    let mut previous = None;
    let mut local_index = 0;
    for source in sources {
        let primary = location_value(source.primary_source(), documents)?;
        let contributing = source
            .contributing_sources()
            .map(|v| location_value(v, documents))
            .collect::<Result<Vec<_>, _>>()?;
        match source.target() {
            PolicySourceTarget::Declaration { id, ordinal } => {
                stable.push(StableSourceProjection {
                    entity_kind: EntityKind::RightOfWayPolicySet,
                    stable_id: stable_id_bytes(*id),
                    typed_ordinal: ordinal.raw(),
                    primary,
                    contributing,
                })
            }
            PolicySourceTarget::Member { owner, kind, .. } => {
                let key = (stable_id_bytes(*owner), kind.code());
                if previous != Some(key) {
                    previous = Some(key);
                    local_index = 0;
                }
                members.push(OwnerLocalProjection {
                    owner_entity_kind: EntityKind::RightOfWayPolicySet,
                    owner_stable_id: key.0,
                    role: key.1 + 33,
                    local_index,
                    primary,
                    contributing,
                });
                local_index += 1;
            }
            PolicySourceTarget::MovementDirection { id } => {
                if !contributing.is_empty() {
                    return Err(PortableEmissionError::PolicySourceMismatch);
                }
                let target = stable
                    .iter_mut()
                    .find(|s| {
                        s.entity_kind == EntityKind::Movement && s.stable_id == stable_id_bytes(*id)
                    })
                    .ok_or(PortableEmissionError::PolicySourceMismatch)?;
                target.contributing.push(primary);
            }
        }
    }
    Ok(())
}
