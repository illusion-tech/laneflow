//! emitter 的 K 配对与完整 RowV1 投影。checker 只复用预算/分配原语，不调用配对或编码。

use super::base::{checked_ordinal_vector_with, checked_u32_with};
use super::*;
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

/// LFSD 构建暂存区的字节计量器，按编译资源上限记账。
pub(in crate::portable_emitter) struct Scratch {
    used: u64,
    limit: u64,
}

impl Scratch {
    /// 创建用量为零、上限为 `limit` 的暂存计量器。
    pub(in crate::portable_emitter) const fn new(limit: u64) -> Self {
        Self { used: 0, limit }
    }
    /// 计入 `bytes` 字节暂存用量；超过上限时返回 `CompileLimitExceeded` 错误。
    pub(in crate::portable_emitter) fn charge(
        &mut self,
        bytes: u64,
    ) -> Result<(), PortableEmissionError> {
        let actual = self
            .used
            .checked_add(bytes)
            .ok_or(PortableEmissionError::ArithmeticOverflow)?;
        if actual > self.limit {
            return Err(PortableEmissionError::CompileLimitExceeded {
                dimension: CompileLimitDimension::StageScratchBytes,
                actual,
                limit: self.limit,
            });
        }
        self.used = actual;
        Ok(())
    }
    /// 释放 `bytes` 字节暂存用量。
    pub(in crate::portable_emitter) fn release(&mut self, bytes: u64) {
        self.used -= bytes;
    }

    /// 返回当前已计入的暂存字节用量。
    #[cfg(test)]
    pub(in crate::portable_emitter) const fn used(&self) -> u64 {
        self.used
    }
}

fn members<'a>(
    index: &ArtifactIndex<'a>,
    scratch: &mut Scratch,
) -> Result<Vec<Member<'a>>, PortableEmissionError> {
    let section = index.view.section(3).ok_or(MISMATCH)?;
    let count = (1..=4).try_fold(0_usize, |n, i| {
        n.checked_add(section.table(i).ok_or(MISMATCH)?.row_count() as usize)
            .ok_or(PortableEmissionError::ArithmeticOverflow)
    })?;
    let mut members = reserved::<Member<'a>>(count, scratch)?;
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
            });
        }
    }
    members.sort_unstable_by(|a, b| a.key().cmp(&b.key()));
    Ok(members)
}

/// 在暂存预算内创建预留 `count` 个元素容量的空 `Vec`。
///
/// 先按元素大小折算字节并记账，再执行精确预留；分配失败返回 `AllocationFailure`。
pub(in crate::portable_emitter) fn reserved<T>(
    count: usize,
    scratch: &mut Scratch,
) -> Result<Vec<T>, PortableEmissionError> {
    let bytes = count
        .checked_mul(core::mem::size_of::<T>())
        .ok_or(PortableEmissionError::ArithmeticOverflow)?;
    scratch.charge(u64::try_from(bytes).map_err(|_| PortableEmissionError::ArithmeticOverflow)?)?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| PortableEmissionError::AllocationFailure)?;
    Ok(values)
}

/// 生成两版制品之间的路权策略局部成员变更行（LFSD 第 7 节）。
///
/// 按策略稳定标识、成员种类与原始 key 配对成员并投影完整载荷；`base` 为 `None`
/// 时全部成员按新增输出。
pub(super) fn policy_changes(
    base: Option<&ArtifactIndex<'_>>,
    target: &ArtifactIndex<'_>,
    scratch: &mut Scratch,
) -> Result<Vec<OwnedRow>, PortableEmissionError> {
    let before = base
        .map(|b| members(b, scratch))
        .transpose()?
        .unwrap_or_default();
    let after = members(target, scratch)?;
    let maximum = before
        .len()
        .checked_add(after.len())
        .ok_or(PortableEmissionError::ArithmeticOverflow)?;
    let mut changes = reserved::<(u8, [u8; 16], u8, &str, OwnedRow)>(maximum, scratch)?;
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
        let bv = b
            .map(|m| project(base.ok_or(MISMATCH)?, *m, scratch))
            .transpose()?;
        let av = a.map(|m| project(target, *m, scratch)).transpose()?;
        if bv == av {
            scratch.release(
                bv.as_ref().map_or(0, |v| v.len() as u64)
                    + av.as_ref().map_or(0, |v| v.len() as u64),
            );
            continue;
        }
        let member = a.or(b).ok_or(MISMATCH)?;
        let op = match (b, a) {
            (None, _) => 0,
            (_, None) => 1,
            _ => 2,
        };
        let mut fields = reserved::<OwnedField>(if op == 2 { 6 } else { 5 }, scratch)?;
        scratch.charge(member.key.len() as u64)?;
        fields.extend([
            field(1, OwnedValue::U8(op)),
            field(2, OwnedValue::StableId128(member.owner)),
            field(3, OwnedValue::U8(member.kind)),
            field(4, OwnedValue::Utf8(member.key.into())),
        ]);
        if let Some(v) = bv {
            fields.push(field(5, OwnedValue::Bytes(v)));
        }
        if let Some(v) = av {
            fields.push(field(6, OwnedValue::Bytes(v)));
        }
        changes.push((
            op,
            member.owner,
            member.kind,
            member.key,
            OwnedRow {
                fields: fields.into_boxed_slice(),
            },
        ));
    }
    changes.sort_unstable_by(|a, b| (a.0, a.1, a.2, a.3).cmp(&(b.0, b.1, b.2, b.3)));
    // 排序记录与返回行缓冲短暂同时存活，两个都计入峰值。
    let mut rows = reserved::<OwnedRow>(changes.len(), scratch)?;
    rows.extend(changes.into_iter().map(|r| r.4));
    Ok(rows)
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

fn project(
    index: &ArtifactIndex<'_>,
    member: Member<'_>,
    scratch: &mut Scratch,
) -> Result<Box<[u8]>, PortableEmissionError> {
    let mut length = 16_usize;
    let mut count = 0_u32;
    for f in member.row.fields().filter(|f| f.tag() >= 3) {
        let size = match reference(member.kind, f.tag()) {
            Some((_, false)) => 18,
            Some((_, true)) => (checked_ordinal_vector_with(member.row, f.tag(), MISMATCH)?.len()
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
        count += 1;
    }
    let mut bytes = reserved::<u8>(length, scratch)?;
    bytes.extend_from_slice(&(length as u64).to_le_bytes());
    bytes.extend_from_slice(&count.to_le_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    for f in member.row.fields().filter(|f| f.tag() >= 3) {
        bytes.extend_from_slice(&f.tag().to_le_bytes());
        if let Some((kind, vector)) = reference(member.kind, f.tag()) {
            bytes.extend_from_slice(&[PortableFieldType::Bytes as u8, 0]);
            if vector {
                let refs = checked_ordinal_vector_with(member.row, f.tag(), MISMATCH)?;
                bytes.extend_from_slice(&(4 + u64::from(refs.len()) * 18).to_le_bytes());
                bytes.extend_from_slice(&refs.len().to_le_bytes());
                for position in 0..refs.len() {
                    bytes.extend_from_slice(&kind.code().to_le_bytes());
                    bytes.extend_from_slice(&index.stable_id(
                        kind,
                        refs.get(position).ok_or(MISMATCH)?,
                        MISMATCH,
                    )?);
                }
            } else {
                bytes.extend_from_slice(&18_u64.to_le_bytes());
                bytes.extend_from_slice(&kind.code().to_le_bytes());
                bytes.extend_from_slice(&index.stable_id(
                    kind,
                    checked_u32_with(member.row, f.tag(), MISMATCH)?,
                    MISMATCH,
                )?);
            }
        } else {
            bytes.extend_from_slice(&[f.field_type() as u8, 0]);
            bytes.extend_from_slice(&(f.value_bytes().len() as u64).to_le_bytes());
            bytes.extend_from_slice(f.value_bytes());
        }
    }
    Ok(bytes.into_boxed_slice())
}
