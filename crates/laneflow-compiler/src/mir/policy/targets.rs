//! 按 subject passage 建立 exact target-cell ranges；未共享的 zone 不产生虚构 cell。
use super::*;
use crate::arena::TableRange;

#[derive(Clone, Copy)]
pub(super) struct TargetCell {
    pub stream: MirParticipantStreamKey,
    pub passage: u32,
}
pub(super) struct TargetRange {
    pub rule: u32,
    pub subject_passage: u32,
    pub targets: TableRange<TargetCell>,
}
pub(super) struct TargetTable {
    pub ranges: Vec<TargetRange>,
    pub cells: Vec<TargetCell>,
    pub bytes: u64,
}

pub(super) fn build(
    unit: &CompilationUnit,
    mir: &MirUnit,
    policy: &MirPolicy,
    prior_bytes: u64,
    prior_records: u64,
) -> Result<TargetTable, DiagnosticBundle> {
    let mut range_count = 0_u64;
    let mut cell_count = 0_u64;
    for rule in &policy.value.streams {
        for source_index in mir.participant_streams[rule.stream.index()]
            .passages
            .as_usize_range()
        {
            range_count = range_count.saturating_add(1);
            visit(mir, rule, source_index, |_| {
                cell_count = cell_count.saturating_add(1);
            });
            super::validation::budget(
                unit,
                mir,
                prior_bytes
                    .saturating_add(range_count.saturating_mul(size_of::<TargetRange>() as u64))
                    .saturating_add(cell_count.saturating_mul(size_of::<TargetCell>() as u64)),
                prior_records
                    .saturating_add(range_count)
                    .saturating_add(cell_count),
            )?;
        }
    }
    let bytes = range_count
        .saturating_mul(size_of::<TargetRange>() as u64)
        .saturating_add(cell_count.saturating_mul(size_of::<TargetCell>() as u64));
    let mut ranges = Vec::with_capacity(range_count as usize);
    let mut cells = Vec::with_capacity(cell_count as usize);
    for (index, rule) in policy.value.streams.iter().enumerate() {
        for source_index in mir.participant_streams[rule.stream.index()]
            .passages
            .as_usize_range()
        {
            let start = cells.len();
            visit(mir, rule, source_index, |cell| cells.push(cell));
            cells[start..].sort_unstable_by(|a, b| {
                // 同一 range 的 zone 固定；按流的完整 Identity 前像规范顺序比较，
                // 不能用摘要字节顺序代替 canonical rank。
                let a = &mir.participant_streams[a.stream.index()];
                let b = &mir.participant_streams[b.stream.index()];
                crate::policy::compare_identity_text(
                    &mir.modules[a.module.index()].authoring_namespace_id,
                    &mir.modules[b.module.index()].authoring_namespace_id,
                )
                .then_with(|| crate::policy::compare_identity_text(&a.stable_key, &b.stable_key))
                .then_with(|| {
                    mir.junctions[a.junction.index()]
                        .stable_id
                        .cmp(&mir.junctions[b.junction.index()].stable_id)
                })
            });
            let targets = TableRange::try_from_usize(start, cells.len() - start).map_err(|_| {
                DiagnosticBundle::single(Diagnostic::compile_limit_exceeded(
                    CompileLimitDimension::RelationOccurrenceCount,
                    u32::MAX.into(),
                    cell_count,
                ))
            })?;
            ranges.push(TargetRange {
                rule: index as u32,
                subject_passage: source_index as u32,
                targets,
            });
        }
    }
    Ok(TargetTable {
        ranges,
        cells,
        bytes,
    })
}
fn visit(
    mir: &MirUnit,
    rule: &StreamRule<MirParticipantStreamKey, MirParticipantClassKey>,
    source: usize,
    mut emit: impl FnMut(TargetCell),
) {
    let zone = mir.conflict_passages[source].conflict_zone;
    for &stream in &rule.yield_to {
        for passage in mir.participant_streams[stream.index()]
            .passages
            .as_usize_range()
        {
            if mir.conflict_passages[passage].conflict_zone == zone {
                emit(TargetCell {
                    stream,
                    passage: passage as u32,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_cells_are_exact_for_each_subject_zone_without_fake_missing_cells() {
        let (unit, mut mir) = super::super::fixture();
        let p = &mir.policies[0];
        let subject = p
            .value
            .streams
            .iter()
            .find(|r| !r.yield_to.is_empty())
            .unwrap();
        let subject_stream = subject.stream;
        let target_stream = subject.yield_to[0];
        let source_index = mir.participant_streams[subject_stream.index()]
            .passages
            .as_usize_range()
            .start;
        let source = &mir.conflict_passages[source_index];
        let copy = |zone| super::super::super::MirConflictPassage {
            conflict_zone: zone,
            entry: source.entry.clone(),
            exit: source.exit.clone(),
            admission_gate: source.admission_gate,
            source_location: source.source_location.clone(),
        };
        let shared = copy(source.conflict_zone);
        let subject_only = copy(MirConflictZoneKey::from_raw(1));
        let mut passages = std::mem::take(&mut mir.conflict_passages).into_vec();
        let start = passages.len();
        passages.extend([shared, subject_only]);
        mir.participant_streams[subject_stream.index()].passages =
            TableRange::try_from_usize(start, 2).unwrap();
        mir.conflict_passages = passages.into_boxed_slice();
        let table = build(&unit, &mir, &mir.policies[0], 0, 0).unwrap();
        let shared = table
            .ranges
            .iter()
            .find(|r| r.subject_passage as usize == start)
            .unwrap();
        let only = table
            .ranges
            .iter()
            .find(|r| r.subject_passage as usize == start + 1)
            .unwrap();
        let cells = &table.cells[shared.targets.as_usize_range()];
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].stream, target_stream);
        assert!(only.targets.as_usize_range().is_empty());
    }
}
