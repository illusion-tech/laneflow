//! 从受检 LFCA 行重建各表内的所有者序号，逐项核对冻结来源与最终 LFSM。
use super::*;
use crate::source_map::station::{StationSource, StationTarget};

type Result<T> = std::result::Result<T, PortableEmissionError>;
type Extent = (EntityKind, u32, u32);
pub(super) fn plan(
    source: &ValidatedSourceMapInput,
    root: RegistryCheckedObjectView<'_>,
) -> Result<Plan> {
    if source.station_sources.is_empty() {
        return Ok(Plan::ZERO);
    }
    Ok(Plan::ZERO
        .array::<&StationSource>(source.station_sources.len())
        .array::<Extent>(table(root, 8, 6)?.row_count() as usize)
        .array::<(u32, u64)>(table(root, 8, 1)?.row_count() as usize)
        .array::<u32>(
            source
                .station_sources
                .iter()
                .map(|s| s.contributors(source).len())
                .max()
                .unwrap_or(0),
        ))
}

pub(super) fn check(
    source: &ValidatedSourceMapInput,
    root: RegistryCheckedObjectView<'_>,
    map: RegistryCheckedObjectView<'_>,
    locations: &[LocationKey<'_>],
    docs: &Documents<'_>,
    pass: &PassBudget<'_>,
) -> Result<()> {
    let mut actual_count = 0;
    for t in 5..11 {
        actual_count += table(root, 8, t)?.row_count() as u64;
    }
    if actual_count != source.station_sources.len() as u64 {
        return Err(MISMATCH);
    }
    let mut source_rows = source.station_sources.iter();
    for r in table(map, 3, 0)?.rows() {
        let role = number(r, 3)? as u8;
        if !(37..=42).contains(&role) {
            continue;
        }
        let s = source_rows.next().ok_or(MISMATCH)?;
        if number(r, 1)? != u64::from(s.kind.code())
            || checked_stable_id_with(r, 2, MISMATCH)? != s.id
            || role != s.role
            || number(r, 4)? != u64::from(s.local)
        {
            return Err(MISMATCH);
        }
        let primary = source_key(s.primary(source), docs)?;
        if locations.get(number(r, 5)? as usize) != Some(&primary) {
            return Err(MISMATCH);
        }
        let mut expected = pass.vec(s.contributors(source).len())?;
        for l in s.contributors(source) {
            expected.push(
                locations
                    .binary_search(&source_key(l, docs)?)
                    .map_err(|_| MISMATCH)? as u32,
            )?;
        }
        expected.sort_unstable();
        expected.dedup();
        let actual = ordinals(r, 6)?;
        if actual.len() as usize != expected.len()
            || expected
                .iter()
                .enumerate()
                .any(|(i, n)| actual.get(i as u32) != Some(*n))
        {
            return Err(MISMATCH);
        }
    }
    if source_rows.next().is_some() {
        return Err(MISMATCH);
    }
    if actual_count == 0 {
        return Ok(());
    }

    let mut sources = pass.vec(source.station_sources.len())?;
    sources.try_extend(source.station_sources.iter())?;
    let key = |s: &StationSource| (s.kind, s.ordinal, s.role, s.local);
    sources.sort_unstable_by_key(|s| key(s));
    if sources.windows(2).any(|w| key(w[0]) >= key(w[1])) {
        return Err(MISMATCH);
    }
    for kind in [
        EntityKind::RoadCorridor,
        EntityKind::RoadSection,
        EntityKind::AuthoringLane,
        EntityKind::FacilityBand,
    ] {
        let mut entries = sources.iter().filter(|s| s.kind == kind).peekable();
        for r in table(root, 2, u32::from(kind.code() - 1))?.rows() {
            let ordinal = number(r, 1)? as u32;
            while entries.peek().is_some_and(|s| s.ordinal == ordinal) {
                if entries.next().ok_or(MISMATCH)?.id != checked_stable_id_with(r, 2, MISMATCH)? {
                    return Err(MISMATCH);
                }
            }
        }
        if entries.next().is_some() {
            return Err(MISMATCH);
        }
    }
    let mut anchors = pass.vec(table(root, 8, 1)?.row_count() as usize)?;
    let mut rows = table(root, 8, 1)?.rows();
    for c in table(root, 8, 0)?.rows() {
        if number(c, 5)? != anchors.len() as u64 {
            return Err(MISMATCH);
        }
        for _ in 0..number(c, 6)? {
            anchors.push((
                number(c, 1)? as u32,
                number(rows.next().ok_or(MISMATCH)?, 1)?,
            ))?;
        }
    }
    if rows.next().is_some() {
        return Err(MISMATCH);
    }
    let mut extents = pass.vec(table(root, 8, 6)?.row_count() as usize)?;
    for role in 37..=42 {
        let mut previous = None;
        let mut local = 0;
        for r in table(root, 8, u32::from(role) - 32)?.rows() {
            let (kind, owner, occurrence) = match role {
                37 => (EntityKind::FacilityBand, number(r, 1)? as u32, 0),
                38 => (
                    EntityKind::from_code(number(r, 1)? as u16).ok_or(MISMATCH)?,
                    number(r, 2)? as u32,
                    0,
                ),
                39 => (EntityKind::AuthoringLane, number(r, 1)? as u32, 0),
                40 => (EntityKind::RoadCorridor, number(r, 1)? as u32, 0),
                _ => *extents.get(number(r, 1)? as usize).ok_or(MISMATCH)?,
            };
            if previous != Some((kind, owner)) {
                if previous.is_some_and(|p| p > (kind, owner)) {
                    return Err(MISMATCH);
                }
                local = 0;
                previous = Some((kind, owner));
            }
            let i = sources
                .binary_search_by_key(&(kind, owner, role, local), |s| key(s))
                .map_err(|_| MISMATCH)?;
            let s = sources[i];
            let node = |tag| -> Result<(u32, u64)> {
                anchors
                    .get(number(r, tag)? as usize)
                    .copied()
                    .ok_or(MISMATCH)
            };
            let ok = match (&s.target, role) {
                (StationTarget::Width, 37 | 39) => local == 0,
                (StationTarget::Surface { stations }, 40) => {
                    node(3)? == (owner, stations[0]) && node(4)? == (owner, stations[1])
                }
                (
                    StationTarget::Named {
                        corridor,
                        extent,
                        stations,
                        occurrence,
                    },
                    38,
                ) => {
                    extents.push((kind, owner, local))?;
                    *occurrence == local
                        && extent.left == number(r, 3)? as u32
                        && extent.kind == number(r, 4)? as u8
                        && &*extent.key == text(r, 5)?
                        && node(6)? == (*corridor, stations[0])
                        && node(7)? == (*corridor, stations[1])
                }
                (
                    StationTarget::Named {
                        extent,
                        occurrence: ord,
                        ..
                    },
                    41 | 42,
                ) if *ord == occurrence => match extent.payload {
                    Some(
                        crate::road_editing::boundary_payload::QuantizedBoundaryPayload::Rule {
                            directions,
                            effect,
                        },
                    ) => {
                        role == 41
                            && u64::from(directions) == number(r, 2)?
                            && u64::from(effect) == number(r, 3)?
                    }
                    Some(p) => {
                        let mut buf = [0; 36];
                        let (shape, n) = p.obstacle_bytes(&mut buf).ok_or(MISMATCH)?;
                        role == 42
                            && u64::from(shape) == number(r, 2)?
                            && r.field_by_tag(3).ok_or(MISMATCH)?.value_bytes() == &buf[..n]
                    }
                    None => false,
                },
                _ => false,
            };
            if !ok {
                return Err(MISMATCH);
            }
            local += 1;
        }
    }
    Ok(())
}
