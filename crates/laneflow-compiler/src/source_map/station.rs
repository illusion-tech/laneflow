use super::*;
use crate::road_editing::boundary::{NamedExtent, RowSource};
use std::sync::Arc;

#[derive(Clone)]
pub(crate) enum StationTarget {
    Width,
    Surface {
        stations: [u64; 2],
    },
    Named {
        corridor: u32,
        extent: Arc<NamedExtent>,
        stations: [u64; 2],
        occurrence: u32,
    },
}

pub(crate) struct StationSource {
    pub kind: EntityKind,
    pub id: [u8; 16],
    pub ordinal: u32,
    pub role: u8,
    pub local: u32,
    pub target: StationTarget,
    primary: SourceLocationRecord,
    contributing: crate::ledger::Frozen<SourceLocationRecord>,
}

impl StationSource {
    pub(crate) fn primary<'a>(
        &'a self,
        map: &'a ValidatedSourceMapInput,
    ) -> SourceLocationView<'a> {
        map.location(&self.primary)
    }
    pub(crate) fn contributors<'a>(
        &'a self,
        map: &'a ValidatedSourceMapInput,
    ) -> impl ExactSizeIterator<Item = SourceLocationView<'a>> {
        self.contributing.iter().map(|l| map.location(l))
    }
    pub(super) fn transfer(&mut self) {
        self.contributing.transfer_to_host();
    }
}

pub(super) fn visit<'a, E>(
    lir: &'a crate::lir::LirUnit,
    corridor: &'a crate::lir::LirRoadCorridor,
    mut f: impl FnMut(EntityKind, [u8; 16], u32, u8, u32, StationTarget, &'a RowSource) -> Result<(), E>,
) -> Result<(), E> {
    let Some(p) = &corridor.cross_section else {
        return Ok(());
    };
    let mut row = |kind, ordinal: u32, role, local, target, source| {
        let i = ordinal as usize;
        let id = match kind {
            EntityKind::RoadCorridor => *lir.road_corridors[i].stable_id.into_untyped().as_bytes(),
            EntityKind::RoadSection => *lir.road_sections[i].stable_id.into_untyped().as_bytes(),
            EntityKind::AuthoringLane => {
                *lir.authoring_lanes[i].stable_id.into_untyped().as_bytes()
            }
            EntityKind::FacilityBand => *lir.facility_bands[i].stable_id.into_untyped().as_bytes(),
            _ => unreachable!("registered station owner"),
        };
        f(kind, id, ordinal, role, local, target, source)
    };
    for w in &p.widths {
        let (k, i) = corridor
            .width_owner(lir, w.member)
            .expect("resolved cross-section member");
        row(
            k,
            i,
            if w.lane.is_some() { 39 } else { 37 },
            0,
            StationTarget::Width,
            &w.source,
        )?;
    }
    for (i, s) in p.surface.iter().enumerate() {
        row(
            EntityKind::RoadCorridor,
            corridor.ordinal.raw(),
            40,
            i as u32,
            StationTarget::Surface {
                stations: s.stations.map(|n| n.0),
            },
            &s.source,
        )?;
    }
    for group in p
        .extents
        .chunk_by(|a, b| a.section_element == b.section_element)
    {
        let (k, o) = corridor
            .extent_owner(lir, group[0].section_element)
            .expect("resolved extent owner");
        let mut next = [0u32; 2];
        for (i, e) in group.iter().enumerate() {
            let target = StationTarget::Named {
                corridor: corridor.ordinal.raw(),
                extent: e.declaration.clone(),
                stations: e.stations.map(|n| n.0),
                occurrence: i as u32,
            };
            row(
                k,
                o,
                38,
                i as u32,
                target.clone(),
                &e.declaration.name_source,
            )?;
            if let Some(s) = &e.declaration.payload_source {
                let j = usize::from(e.declaration.kind - 1);
                row(k, o, 41 + j as u8, next[j], target, s)?;
                next[j] += 1;
            }
        }
    }
    Ok(())
}

pub(super) fn freeze(
    lir: &LirFreezeOutput,
    mir: &MirUnit,
    unit: &CompilationUnit,
    n: usize,
    pass: &crate::ledger::PassBudget<'_>,
) -> Result<crate::ledger::Frozen<StationSource>, DiagnosticBundle> {
    let mut records = pass.vec(n)?;
    for &key in lir.road_corridors.stage_keys_in_lir_order() {
        let c = &lir.lir.road_corridors[lir.road_corridors.ordinal(key).index()];
        let module = mir.road_corridors[key.index()].module.raw();
        visit(&lir.lir, c, |kind, id, ordinal, role, local, target, s| {
            let resolve = |l| {
                unit.resolve_source_location_for_module(module, l)
                    .map(SourceLocationRecord::from)
            };
            let mut contributing = pass.vec(s.contributing.len())?;
            for l in &s.contributing {
                contributing.push(resolve(l)?)?;
            }
            records.push(StationSource {
                kind,
                id,
                ordinal,
                role,
                local,
                target,
                primary: resolve(&s.primary)?,
                contributing: contributing.freeze()?,
            })?;
            Ok::<_, DiagnosticBundle>(())
        })?;
    }
    records.sort_unstable_by_key(|s| (s.kind, s.id, s.role, s.local));
    Ok(records.freeze()?)
}
