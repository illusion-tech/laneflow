//! 从具体 LFCA 的站位表解析稳定成员，消除表内行号后比较两个修订。
use super::base::{
    checked_ordinal_vector_with, checked_record_vector_with, checked_u8_with, checked_u16_with,
    checked_u32_with,
};
use super::changes::{ChangeBounds, add};
use super::*;
use crate::ledger::{BoundedVec, PassBudget, Plan};
type Result<T> = std::result::Result<T, PortableEmissionError>;
const INVALID: PortableEmissionError = PortableEmissionError::StationDiffMismatch;

#[derive(Clone, Copy)]
struct Member {
    kind: u8,
    key: Slots,
    value: Slots,
}

#[derive(Clone, Copy)]
pub(super) struct Change {
    pub op: u8,
    pub kind: u8,
    pub key: Slots,
    pub before: Option<Slots>,
    pub after: Option<Slots>,
}

pub(super) fn plan(
    base: Option<RegistryCheckedObjectView<'_>>,
    target: RegistryCheckedObjectView<'_>,
    bounds: &mut ChangeBounds,
) -> Result<Plan> {
    let mut scratch = Plan::ZERO;
    let mut anchors = 0;
    for view in base.into_iter().chain([target]) {
        let section = view.section(8).ok_or(INVALID)?;
        for (i, size) in [38usize, 30, 37, 44, 46, 54, 125].into_iter().enumerate() {
            let n = section.table(i as u32).ok_or(INVALID)?.row_count() as usize;
            add(&mut bounds.station, n)?;
            add(
                &mut bounds.staged,
                n.checked_mul(size)
                    .ok_or(PortableEmissionError::ArithmeticOverflow)?,
            )?;
            if i == 1 {
                anchors = anchors.max(n)
            }
        }
    }
    scratch = scratch
        .array::<Member>(bounds.station)
        .array::<(u32, [u8; 8])>(anchors);
    Ok(scratch)
}

fn field(row: RegistryCheckedRowView<'_>, tag: u16) -> Result<&[u8]> {
    Ok(row.field_by_tag(tag).ok_or(INVALID)?.value_bytes())
}

fn reference(index: &ArtifactIndex<'_>, kind: EntityKind, ordinal: u32) -> Result<[u8; 18]> {
    let mut bytes = [0; 18];
    bytes[..2].copy_from_slice(&kind.code().to_le_bytes());
    bytes[2..].copy_from_slice(&index.stable_id(kind, ordinal, INVALID)?);
    Ok(bytes)
}

fn push_member(
    bytes: &mut BoundedVec<u8>,
    members: &mut BoundedVec<Member>,
    kind: u8,
    key: &[&[u8]],
    value: &[&[u8]],
) -> Result<()> {
    let mut append = |parts: &[&[u8]]| -> Result<Slots> {
        let start = bytes.len();
        for part in parts {
            bytes.extend_from_slice(part)?;
        }
        Ok(Slots {
            start,
            end: bytes.len(),
        })
    };
    members.push(Member {
        kind,
        key: append(key)?,
        value: append(value)?,
    })?;
    Ok(())
}

fn project(
    index: &ArtifactIndex<'_>,
    pass: &PassBudget<'_>,
    bytes: &mut BoundedVec<u8>,
) -> Result<BoundedVec<Member>> {
    use EntityKind::{AuthoringLane, FacilityBand, LaneEdge, RoadCorridor, RoadSection};
    let section = index.view.section(8).ok_or(INVALID)?;
    let table = |i| section.table(i).ok_or(INVALID);
    let count = section.tables().try_fold(0usize, |n, t| {
        n.checked_add(t.row_count() as usize)
            .ok_or(PortableEmissionError::ArithmeticOverflow)
    })?;
    let mut members = pass.vec(count)?;
    let mut emit =
        |kind, key: &[&[u8]], value: &[&[u8]]| push_member(bytes, &mut members, kind, key, value);
    let num = |r, t| checked_u32_with(r, t, INVALID);
    let anchor_table = table(1)?;
    let mut anchors = pass.vec(anchor_table.row_count() as usize)?;
    let mut anchor_rows = anchor_table.rows();
    for row in table(0)?.rows() {
        let c = num(row, 1)?;
        let id = reference(index, RoadCorridor, c)?;
        emit(1, &[&id], &[field(row, 2)?, field(row, 3)?, field(row, 4)?])?;
        if num(row, 5)? as usize != anchors.len() {
            return Err(INVALID);
        }
        for _ in 0..num(row, 6)? {
            let a = anchor_rows.next().ok_or(INVALID)?;
            let s = field(a, 1)?;
            anchors.push((c, <[u8; 8]>::try_from(s).map_err(|_| INVALID)?))?;
            emit(2, &[&id, s], &[field(a, 2)?])?;
        }
    }
    if anchor_rows.next().is_some() {
        return Err(INVALID);
    }
    let anchor = |ordinal: u32, c: u32| {
        anchors
            .get(ordinal as usize)
            .filter(|a| a.0 == c)
            .map(|a| &a.1[..])
            .ok_or(INVALID)
    };
    let mut edges = table(3)?.rows().enumerate();
    let mut spans = table(4)?.rows().enumerate();
    for lane in table(2)?.rows() {
        let l = num(lane, 1)?;
        let lr = index.entity_row(AuthoringLane, l, INVALID)?;
        let sr = index.entity_row(RoadSection, num(lr, 3)?, INVALID)?;
        let c = num(sr, 3)?;
        let lid = reference(index, AuthoringLane, l)?;
        emit(
            3,
            &[&lid],
            &[&reference(index, RoadCorridor, c)?, field(lane, 2)?],
        )?;
        let chain = checked_ordinal_vector_with(lr, 4, INVALID)?;
        if chain.len() != num(lane, 4)? {
            return Err(INVALID);
        }
        for i in 0..chain.len() {
            let (j, e) = edges.next().ok_or(INVALID)?;
            let ordinal = num(e, 1)?;
            if j != num(lane, 3)? as usize + i as usize || Some(ordinal) != chain.get(i) {
                return Err(INVALID);
            }
            let eid = reference(index, LaneEdge, ordinal)?;
            emit(4, &[&eid], &[&lid, anchor(num(e, 2)?, c)?])?;
            for k in 0..num(e, 4)? {
                let (j, s) = spans.next().ok_or(INVALID)?;
                if j != num(e, 3)? as usize + k as usize {
                    return Err(INVALID);
                }
                emit(
                    5,
                    &[&eid, anchor(num(s, 1)?, c)?],
                    &[field(s, 2)?, field(s, 3)?, field(s, 4)?, field(s, 5)?],
                )?;
            }
        }
    }
    if edges.next().is_some() || spans.next().is_some() {
        return Err(INVALID);
    }
    for row in table(5)?.rows() {
        let b = num(row, 1)?;
        let c = num(index.entity_row(FacilityBand, b, INVALID)?, 3)?;
        emit(
            6,
            &[&reference(index, FacilityBand, b)?],
            &[
                anchor(num(row, 2)?, c)?,
                anchor(num(row, 3)?, c)?,
                field(row, 4)?,
                field(row, 5)?,
                field(row, 6)?,
                field(row, 7)?,
                field(row, 8)?,
            ],
        )?;
    }
    for row in table(6)?.rows() {
        let owner = EntityKind::from_code(checked_u16_with(row, 1, INVALID)?).ok_or(INVALID)?;
        let ordinal = num(row, 2)?;
        let r = index.entity_row(owner, ordinal, INVALID)?;
        let left = num(row, 3)?;
        let right = left.checked_add(1).ok_or(INVALID)?;
        let (c, a, b) = match owner {
            RoadCorridor => {
                let elements = checked_record_vector_with(r, 4, INVALID)?;
                let element = |i| -> Result<[u8; 18]> {
                    let e = elements.row(i).ok_or(INVALID)?;
                    let kind = match checked_u8_with(e, 1, INVALID)? {
                        0 => RoadSection,
                        1 => FacilityBand,
                        _ => return Err(INVALID),
                    };
                    reference(index, kind, num(e, 2)?)
                };
                (ordinal, element(left)?, element(right)?)
            }
            RoadSection => {
                let lanes = checked_ordinal_vector_with(r, 5, INVALID)?;
                (
                    num(r, 3)?,
                    reference(index, AuthoringLane, lanes.get(left).ok_or(INVALID)?)?,
                    reference(index, AuthoringLane, lanes.get(right).ok_or(INVALID)?)?,
                )
            }
            _ => return Err(INVALID),
        };
        let name = field(row, 5)?;
        emit(
            7,
            &[
                &reference(index, owner, ordinal)?,
                &a,
                &b,
                field(row, 4)?,
                &[u8::try_from(name.len()).map_err(|_| INVALID)?],
                name,
            ],
            &[anchor(num(row, 6)?, c)?, anchor(num(row, 7)?, c)?],
        )?;
    }
    let key = |m: &Member| (m.kind, m.key.of(bytes));
    members.sort_unstable_by(|a, b| key(a).cmp(&key(b)));
    if members.windows(2).any(|w| key(&w[0]) == key(&w[1])) {
        return Err(INVALID);
    }
    Ok(members)
}

pub(super) fn changes(
    base: Option<&ArtifactIndex<'_>>,
    target: &ArtifactIndex<'_>,
    pass: &PassBudget<'_>,
    out: &mut BoundedVec<Change>,
    bytes: &mut BoundedVec<u8>,
) -> Result<()> {
    use std::cmp::Ordering;
    let before = if let Some(index) = base {
        project(index, pass, bytes)?
    } else {
        pass.vec(0)?
    };
    let after = project(target, pass, bytes)?;
    let key = |m: Member| (m.kind, m.key.of(bytes));
    let (mut i, mut j) = (0, 0);
    while i < before.len() || j < after.len() {
        let (op, m, b, a) = match (before.get(i).copied(), after.get(j).copied()) {
            (Some(b), Some(a)) if key(b) == key(a) => {
                i += 1;
                j += 1;
                if b.value.of(bytes) == a.value.of(bytes) {
                    continue;
                }
                (2, a, Some(b.value), Some(a.value))
            }
            (Some(b), a) if a.is_none_or(|v| key(b).cmp(&key(v)) == Ordering::Less) => {
                i += 1;
                (1, b, Some(b.value), None)
            }
            (_, Some(a)) => {
                j += 1;
                (0, a, None, Some(a.value))
            }
            _ => return Err(INVALID),
        };
        out.push(Change {
            op,
            kind: m.kind,
            key: m.key,
            before: b,
            after: a,
        })?;
    }
    out.sort_unstable_by(|a, b| {
        (a.op, a.kind, a.key.of(bytes)).cmp(&(b.op, b.kind, b.key.of(bytes)))
    });
    Ok(())
}

/// 根据两端制品重新解析，再逐行核对操作、稳定键、两侧值和行序。
pub(super) fn check(
    base: Option<&ArtifactIndex<'_>>,
    target: &ArtifactIndex<'_>,
    diff: RegistryCheckedObjectView<'_>,
    pass: &PassBudget<'_>,
) -> Result<()> {
    let mut bounds = ChangeBounds::default();
    plan(base.map(|i| i.view), target.view, &mut bounds)?;
    let mut rows = pass.vec(bounds.station)?;
    let mut bytes = pass.vec(bounds.staged)?;
    changes(base, target, pass, &mut rows, &mut bytes)?;
    let table = diff.section(7).and_then(|s| s.table(0)).ok_or(INVALID)?;
    if rows.len() != table.row_count() as usize {
        return Err(INVALID);
    }
    for (expected, actual) in rows.iter().zip(table.rows()) {
        if expected.op != checked_u8_with(actual, 1, INVALID)?
            || expected.kind != checked_u8_with(actual, 2, INVALID)?
            || expected.key.of(&bytes) != field(actual, 3)?
        {
            return Err(INVALID);
        }
        for (tag, side) in [(4, expected.before), (5, expected.after)] {
            if side.map(|s| s.of(&bytes)) != actual.field_by_tag(tag).map(|f| f.value_bytes()) {
                return Err(INVALID);
            }
        }
    }
    Ok(())
}
