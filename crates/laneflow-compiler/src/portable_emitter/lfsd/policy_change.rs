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

pub(in crate::portable_emitter) struct Scratch {
    used: u64,
    limit: u64,
}

impl Scratch {
    pub(in crate::portable_emitter) const fn new(limit: u64) -> Self {
        Self { used: 0, limit }
    }
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
    pub(in crate::portable_emitter) fn release(&mut self, bytes: u64) {
        self.used -= bytes;
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

pub(super) fn policy_changes(
    base: Option<&ArtifactIndex<'_>>,
    target: &ArtifactIndex<'_>,
    scratch_limit: u64,
) -> Result<Vec<OwnedRow>, PortableEmissionError> {
    let mut scratch = Scratch::new(scratch_limit);
    let before = base
        .map(|b| members(b, &mut scratch))
        .transpose()?
        .unwrap_or_default();
    let after = members(target, &mut scratch)?;
    let maximum = before
        .len()
        .checked_add(after.len())
        .ok_or(PortableEmissionError::ArithmeticOverflow)?;
    let mut changes = reserved::<(u8, [u8; 16], u8, &str, OwnedRow)>(maximum, &mut scratch)?;
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
            .map(|m| project(base.ok_or(MISMATCH)?, *m, &mut scratch))
            .transpose()?;
        let av = a.map(|m| project(target, *m, &mut scratch)).transpose()?;
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
        let mut fields = reserved::<OwnedField>(if op == 2 { 6 } else { 5 }, &mut scratch)?;
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
    let mut rows = reserved::<OwnedRow>(changes.len(), &mut scratch)?;
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
