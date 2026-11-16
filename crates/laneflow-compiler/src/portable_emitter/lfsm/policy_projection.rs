//! emitter 从受检来源按 owner/kind/key 独立排序；不从实际 LFSM 行反推成员。

use super::*;
use crate::PolicySourceTarget;

/// 把受检来源中的策略来源按 target 排序、查重后追加为 LFSM 的稳定来源与所有者局部
/// 来源投影；target 重复或成员来源不闭合时返回 `PolicySourceMismatch`。
///
/// `sources` 是调用方按策略来源数授予的排序表；贡献来源追加到 `contributing`。移动方向
/// 来源不新增投影，而是作为既有 Movement 稳定来源的追加贡献来源。
pub(super) fn append_policy_sources<'a>(
    source_map: &'a crate::ValidatedSourceMapInput,
    documents: &DocumentOrdinals<'_>,
    sources: &mut BoundedVec<crate::PolicySourceView<'a>>,
    stable: &mut BoundedVec<StableSourceProjection<'a>>,
    members: &mut BoundedVec<OwnerLocalProjection<'a>>,
    contributing: &mut BoundedVec<LocationValue<'a>>,
) -> Result<(), PortableEmissionError> {
    sources.try_extend(source_map.policy_sources())?;
    sources.sort_unstable_by(|a, b| a.target().cmp(b.target()));
    if sources.windows(2).any(|w| w[0].target() == w[1].target()) {
        return Err(PortableEmissionError::PolicySourceMismatch);
    }
    let mut previous = None;
    let mut local_index = 0;
    // 调用方已按 kind/id/ordinal 排序；后续追加的 policy 不参与既有实体查找。
    let existing_stable_count = stable.len();
    for source in sources.iter() {
        let primary = location_value(source.primary_source(), documents)?;
        let start = contributing.len();
        for location in source.contributing_sources() {
            contributing.push(location_value(location, documents)?)?;
        }
        let contributed = Slots {
            start,
            end: contributing.len(),
        };
        match source.target() {
            PolicySourceTarget::Declaration { id, ordinal } => {
                stable.push(StableSourceProjection {
                    entity_kind: EntityKind::RightOfWayPolicySet,
                    stable_id: stable_id_bytes(*id),
                    typed_ordinal: ordinal.raw(),
                    primary,
                    contributing: contributed,
                    direction: None,
                    set: Slots::default(),
                })?;
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
                    contributing: contributed,
                    set: Slots::default(),
                    derived_set: Slots::default(),
                })?;
                local_index += 1;
            }
            PolicySourceTarget::MovementDirection { id } => {
                if !contributed.is_empty() {
                    return Err(PortableEmissionError::PolicySourceMismatch);
                }
                let index = stable[..existing_stable_count]
                    .binary_search_by_key(&(EntityKind::Movement, stable_id_bytes(*id)), |s| {
                        (s.entity_kind, s.stable_id)
                    })
                    .map_err(|_| PortableEmissionError::PolicySourceMismatch)?;
                stable[index].direction = Some(primary);
            }
        }
    }
    Ok(())
}
