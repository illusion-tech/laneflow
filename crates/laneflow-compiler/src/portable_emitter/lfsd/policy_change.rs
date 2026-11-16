//! emitter 的 K 配对与完整 RowV1 投影。checker 只复用预算/分配原语，不调用配对或编码。

use super::base::{checked_ordinal_vector_with, checked_u32_with};
use super::changes::{ChangeBounds, PolicyChange, add, stage};
use super::*;
use crate::ledger::{BoundedVec, PassBudget, Plan};
use laneflow_static_contract::PortableFieldType;

const MISMATCH: PortableEmissionError = PortableEmissionError::InternalBindingMismatch;

#[derive(Clone, Copy)]
struct Member<'a> {
    owner: [u8; 16],
    kind: u8,
    key: &'a str,
    row: RegistryCheckedRowView<'a>,
}

impl Member<'_> {
    fn key(&self) -> ([u8; 16], u8, &str) {
        (self.owner, self.kind, self.key)
    }
}

fn member_count(index: &ArtifactIndex<'_>) -> Result<usize, PortableEmissionError> {
    let section = index.view.section(3).ok_or(MISMATCH)?;
    (1..=4).try_fold(0_usize, |n, i| {
        n.checked_add(section.table(i).ok_or(MISMATCH)?.row_count() as usize)
            .ok_or(PortableEmissionError::ArithmeticOverflow)
    })
}

fn members<'a>(
    index: &ArtifactIndex<'a>,
    pass: &PassBudget<'_>,
) -> Result<BoundedVec<Member<'a>>, PortableEmissionError> {
    let section = index.view.section(3).ok_or(MISMATCH)?;
    let mut members = pass.vec::<Member<'a>>(member_count(index)?)?;
    for kind in 0..4_u8 {
        for row in section.table(u32::from(kind) + 1).ok_or(MISMATCH)?.rows() {
            let owner = index.stable_id(
                EntityKind::RightOfWayPolicySet,
                checked_u32_with(row, 1, MISMATCH)?,
                MISMATCH,
            )?;
            let RegistryCheckedFieldValue::Utf8(key) =
                row.field_by_tag(2).ok_or(MISMATCH)?.value()?
            else {
                return Err(MISMATCH);
            };
            members.push(Member {
                owner,
                kind,
                key,
                row,
            })?;
        }
    }
    members.sort_unstable_by(|a, b| a.key().cmp(&b.key()));
    Ok(members)
}

/// 策略局部成员变化（第 7 节）的容量与暂存字节上界，以及两侧成员表的计划。
///
/// 变化至多为两侧成员数之和；暂存字节至多为两侧全部成员完整投影之和。
pub(super) fn policy_change_bounds(
    base: Option<&ArtifactIndex<'_>>,
    target: &ArtifactIndex<'_>,
    bounds: &mut ChangeBounds,
) -> Result<Plan, PortableEmissionError> {
    let mut plan = Plan::ZERO;
    for index in base.into_iter().chain([target]) {
        let count = member_count(index)?;
        add(&mut bounds.policy, count)?;
        plan = plan.array::<Member<'_>>(count);
        let section = index.view.section(3).ok_or(MISMATCH)?;
        for kind in 0..4_u8 {
            for row in section.table(u32::from(kind) + 1).ok_or(MISMATCH)?.rows() {
                add(&mut bounds.staged, projection_len(kind, row)?)?;
            }
        }
    }
    Ok(plan)
}

/// 写入两版制品之间的路权策略局部成员变化（LFSD 第 7 节）。
///
/// 按策略稳定标识、成员种类与原始 key 配对成员并投影完整载荷；`base` 为 `None`
/// 时全部成员按新增输出。两侧成员表在本函数内授予并释放；前后投影相同的成员不保留
/// 暂存字节。
pub(super) fn policy_changes<'a>(
    base: Option<&ArtifactIndex<'a>>,
    target: &ArtifactIndex<'a>,
    pass: &PassBudget<'_>,
    changes: &mut BoundedVec<PolicyChange<'a>>,
    staged: &mut BoundedVec<u8>,
) -> Result<(), PortableEmissionError> {
    let before_members = base.map(|b| members(b, pass)).transpose()?;
    let before: &[Member<'a>] = before_members.as_deref().unwrap_or_default();
    let after = members(target, pass)?;
    let (mut i, mut j) = (0, 0);
    while i < before.len() || j < after.len() {
        let b = before.get(i);
        let a = after.get(j);
        let ordering = match (b, a) {
            (Some(b), Some(a)) => b.key().cmp(&a.key()),
            (Some(_), None) => core::cmp::Ordering::Less,
            _ => core::cmp::Ordering::Greater,
        };
        let (b, a) = match ordering {
            core::cmp::Ordering::Less => {
                i += 1;
                (b, None)
            }
            core::cmp::Ordering::Greater => {
                j += 1;
                (None, a)
            }
            core::cmp::Ordering::Equal => {
                i += 1;
                j += 1;
                (b, a)
            }
        };
        let mark = staged.len();
        let bv = b
            .map(|m| project(base.ok_or(MISMATCH)?, *m, staged))
            .transpose()?;
        let av = a.map(|m| project(target, *m, staged)).transpose()?;
        if bv.map(|v| v.of(staged)) == av.map(|v| v.of(staged)) {
            staged.truncate(mark);
            continue;
        }
        let member = a.or(b).ok_or(MISMATCH)?;
        let op = match (b, a) {
            (None, _) => 0,
            (_, None) => 1,
            _ => 2,
        };
        changes.push(PolicyChange {
            op,
            owner: member.owner,
            kind: member.kind,
            key: member.key,
            before: bv,
            after: av,
        })?;
    }
    changes.sort_unstable_by(|a, b| {
        (a.op, a.owner, a.kind, a.key).cmp(&(b.op, b.owner, b.kind, b.key))
    });
    Ok(())
}

fn reference(kind: u8, tag: u16) -> Option<(EntityKind, bool)> {
    match (kind, tag) {
        (2, 3) => Some((EntityKind::ParticipantStream, false)),
        (3, 3) => Some((EntityKind::ManeuverGate, false)),
        (2 | 3, 4) => Some((EntityKind::ParticipantClass, true)),
        (2, 6) => Some((EntityKind::ParticipantStream, true)),
        _ => None,
    }
}

/// 成员完整投影的精确字节数：长度前缀、字段数、保留字与逐字段（标签、类型、长度、值）。
fn projection_len(
    kind: u8,
    row: RegistryCheckedRowView<'_>,
) -> Result<usize, PortableEmissionError> {
    let mut length = 16_usize;
    for f in row.fields().filter(|f| f.tag() >= 3) {
        let size = match reference(kind, f.tag()) {
            Some((_, false)) => 18,
            Some((_, true)) => (checked_ordinal_vector_with(row, f.tag(), MISMATCH)?.len()
                as usize)
                .checked_mul(18)
                .and_then(|v| v.checked_add(4))
                .ok_or(PortableEmissionError::ArithmeticOverflow)?,
            None => f.value_bytes().len(),
        };
        length = length
            .checked_add(12)
            .and_then(|v| v.checked_add(size))
            .ok_or(PortableEmissionError::ArithmeticOverflow)?;
    }
    Ok(length)
}

fn project(
    index: &ArtifactIndex<'_>,
    member: Member<'_>,
    staged: &mut BoundedVec<u8>,
) -> Result<Slots, PortableEmissionError> {
    let length = projection_len(member.kind, member.row)?;
    let count = u32::try_from(member.row.fields().filter(|f| f.tag() >= 3).count())
        .map_err(|_| PortableEmissionError::ArithmeticOverflow)?;
    stage(staged, |bytes| {
        bytes.extend_from_slice(&(length as u64).to_le_bytes())?;
        bytes.extend_from_slice(&count.to_le_bytes())?;
        bytes.extend_from_slice(&0_u32.to_le_bytes())?;
        for f in member.row.fields().filter(|f| f.tag() >= 3) {
            bytes.extend_from_slice(&f.tag().to_le_bytes())?;
            if let Some((kind, vector)) = reference(member.kind, f.tag()) {
                bytes.extend_from_slice(&[PortableFieldType::Bytes as u8, 0])?;
                if vector {
                    let refs = checked_ordinal_vector_with(member.row, f.tag(), MISMATCH)?;
                    bytes.extend_from_slice(&(4 + u64::from(refs.len()) * 18).to_le_bytes())?;
                    bytes.extend_from_slice(&refs.len().to_le_bytes())?;
                    for position in 0..refs.len() {
                        bytes.extend_from_slice(&kind.code().to_le_bytes())?;
                        bytes.extend_from_slice(&index.stable_id(
                            kind,
                            refs.get(position).ok_or(MISMATCH)?,
                            MISMATCH,
                        )?)?;
                    }
                } else {
                    bytes.extend_from_slice(&18_u64.to_le_bytes())?;
                    bytes.extend_from_slice(&kind.code().to_le_bytes())?;
                    bytes.extend_from_slice(&index.stable_id(
                        kind,
                        checked_u32_with(member.row, f.tag(), MISMATCH)?,
                        MISMATCH,
                    )?)?;
                }
            } else {
                bytes.extend_from_slice(&[f.field_type() as u8, 0])?;
                bytes.extend_from_slice(&(f.value_bytes().len() as u64).to_le_bytes())?;
                bytes.extend_from_slice(f.value_bytes())?;
            }
        }
        Ok::<_, PortableEmissionError>(())
    })
}
