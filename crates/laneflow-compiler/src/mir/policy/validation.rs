//! 全部声明的合法性与实际 Access 准入行的静态解析。
use super::*;
use super::{
    access::AccessIndex, passages::PassageIndex, protected::ProtectedIndex, work::WorkBudget,
};
use crate::{
    GateInterpretation as I, GateProhibition as P, ManeuverDirection, PolicyViolation as V,
};

pub(super) fn budget(
    unit: &CompilationUnit,
    mir: &MirUnit,
    scratch: u64,
    records: u64,
) -> Result<(), DiagnosticBundle> {
    crate::policy::check_budget(
        &unit.limits,
        scratch,
        unit.controlled_live_bytes
            .saturating_add(mir.controlled_live_bytes)
            .saturating_add(scratch),
    )?;
    let limit = unit
        .limits
        .value(CompileLimitDimension::RelationOccurrenceCount);
    if records > limit {
        return Err(DiagnosticBundle::single(
            Diagnostic::compile_limit_exceeded(
                CompileLimitDimension::RelationOccurrenceCount,
                limit,
                records,
            ),
        ));
    }
    Ok(())
}
pub(super) fn class_depth(
    mir: &MirUnit,
    ancestor: MirParticipantClassKey,
    class: MirParticipantClassKey,
) -> Option<u32> {
    let ancestor = &mir.participant_classes[ancestor.index()];
    let class = &mir.participant_classes[class.index()];
    (ancestor.subtree_enter <= class.subtree_enter && class.subtree_enter < ancestor.subtree_exit)
        .then_some(ancestor.depth)
}
fn specificity(
    mir: &MirUnit,
    selectors: &Option<Box<[MirParticipantClassKey]>>,
    class: MirParticipantClassKey,
    work: &mut WorkBudget,
) -> Result<Option<u64>, DiagnosticBundle> {
    work.charge(1 + selectors.as_ref().map_or(0, |v| v.len()) as u64)?;
    Ok(selectors.as_ref().map_or(Some(0), |v| {
        v.iter()
            .filter_map(|&a| class_depth(mir, a, class))
            .max()
            .map(|d| u64::from(d) + 1)
    }))
}
fn fail(
    unit: &CompilationUnit,
    p: &MirPolicy,
    member: Option<&str>,
    violation: V,
) -> DiagnosticBundle {
    let TypedAstDeclaration::RightOfWayPolicySet(source) =
        &unit.modules[p.origin.module as usize].declarations[p.origin.declaration as usize]
    else {
        unreachable!("bound origin");
    };
    let location = member
        .and_then(|key| {
            source
                .stream_rules
                .iter()
                .find(|r| r.key.as_ref() == key)
                .map(|r| &r.source.primary)
                .or_else(|| {
                    source
                        .gate_rules
                        .iter()
                        .find(|r| r.key.as_ref() == key)
                        .map(|r| &r.source.primary)
                })
        })
        .unwrap_or(&source.header.span);
    crate::policy::error(&p.value.key, member, violation, location)
}

#[derive(Clone, Copy)]
struct GateCell {
    gate: MirManeuverGateKey,
    profile: u32,
    rule: u32,
}
#[derive(Clone, Copy)]
struct StreamCell {
    stream: MirParticipantStreamKey,
    profile: u32,
    rule: u32,
}
#[derive(Clone, Copy)]
struct RuleIndex {
    owner: u32,
    rule: u32,
}
#[derive(Clone, Copy)]
pub(super) struct Coverage {
    pub(super) gate: MirManeuverGateKey,
    pub(super) zone: MirConflictZoneKey,
}
fn owner_rules(entries: &[RuleIndex], owner: u32) -> &[RuleIndex] {
    &entries[entries.partition_point(|v| v.owner < owner)
        ..entries.partition_point(|v| v.owner <= owner)]
}
pub(crate) fn validate(unit: &CompilationUnit, mir: &mut MirUnit) -> Result<(), DiagnosticBundle> {
    if mir.policies.is_empty() {
        return Ok(());
    }
    let mut work = WorkBudget::new(&unit.limits);
    let passages = PassageIndex::build(unit, mir)?;
    let lamp_bytes = mir.maneuver_gates.len() as u64 + passages.bytes();
    budget(unit, mir, lamp_bytes, mir.conflict_passages.len() as u64)?;
    let mut lamps = vec![0_u8; mir.maneuver_gates.len()];
    // HIR 已拒绝不同的 Access jurisdiction/version；只取唯一规范身份。
    let regulation = mir
        .access_rules
        .iter()
        .find_map(|rule| rule.regulation.as_ref());
    for p in &mir.policies {
        work.charge(1)?;
        if let Some(reg) = regulation
            && (reg.jurisdiction != p.value.regulation.jurisdiction
                || reg.version != p.value.regulation.version)
        {
            return Err(fail(unit, p, None, V::RegulationMismatch));
        }
        for r in &p.value.gates {
            work.charge(1)?;
            let gate = &mir.maneuver_gates[r.gate.index()];
            let unbound = matches!(gate.signal_control, MirSignalControl::None);
            if (r.interpretation == I::Uncontrolled) != unbound
                || (unbound && r.prohibition == P::OnRed)
            {
                return Err(fail(unit, p, Some(&r.key), V::SignalBinding));
            }
            let lamp = match r.interpretation {
                I::CnCircularRightTurn => 1,
                I::DirectionalRightProtected | I::DirectionalRightPermissive => 2,
                _ => 0,
            };
            if lamp != 0 {
                let path = &mir.maneuver_paths[gate.maneuver_path.index()];
                if mir.movements[path.movement.index()].turn_direction
                    != Some(ManeuverDirection::Right)
                {
                    return Err(fail(unit, p, Some(&r.key), V::RightTurnRequired));
                }
                if lamps[r.gate.index()] != 0 && lamps[r.gate.index()] != lamp {
                    return Err(fail(unit, p, Some(&r.key), V::LampTypeConflict));
                }
                lamps[r.gate.index()] = lamp;
            }
        }
        for r in &p.value.streams {
            work.charge(1)?;
            for &target in &r.yield_to {
                work.charge(1)?;
                if target == r.stream {
                    return Err(fail(unit, p, Some(&r.key), V::SelfYield));
                }
                if !passages.shares_zone(r.stream, target, &mut work)? {
                    return Err(fail(unit, p, Some(&r.key), V::DisjointYield));
                }
            }
        }
    }
    drop(lamps);
    let access = AccessIndex::build(
        unit,
        mir,
        passages.bytes(),
        mir.conflict_passages.len() as u64,
        &mut work,
    )?;
    // 只计实际可进入行；不预分配 policy × gate/stream × profile 上界。
    let mut gate_count = 0_u64;
    let mut stream_count = 0_u64;
    let coverage_bytes = (mir.conflict_passages.len() as u64)
        .saturating_mul(size_of::<Coverage>() as u64)
        .saturating_add(mir.maneuver_gates.len() as u64)
        .saturating_add(
            (mir.participant_streams.len() as u64).saturating_mul(size_of::<Option<i32>>() as u64),
        );
    let fixed_bytes = passages
        .bytes()
        .saturating_add(access.bytes())
        .saturating_add(coverage_bytes);
    let fixed_records = (mir.conflict_passages.len() as u64)
        .saturating_mul(2)
        .saturating_add(access.records());
    let mut protected = ProtectedIndex::build(unit, mir, fixed_bytes, fixed_records)?;
    let fixed_bytes = fixed_bytes.saturating_add(protected.bytes());
    let fixed_records = fixed_records.saturating_add(protected.records());
    let check_rows = |gates: u64, streams: u64| {
        budget(
            unit,
            mir,
            fixed_bytes
                .saturating_add(gates.saturating_mul(size_of::<GateCell>() as u64))
                .saturating_add(streams.saturating_mul(size_of::<StreamCell>() as u64)),
            fixed_records.saturating_add(gates).saturating_add(streams),
        )
    };
    check_rows(0, 0)?;
    for gate in &mir.maneuver_gates {
        for profile in &mir.vehicle_profiles {
            if access.path_allows(
                mir,
                gate.maneuver_path,
                profile.participant_class,
                &mut work,
            )? {
                gate_count = gate_count.saturating_add(1);
                check_rows(gate_count, stream_count)?;
            }
        }
    }
    for stream in &mir.participant_streams {
        for profile in &mir.vehicle_profiles {
            if access.path_allows(
                mir,
                stream.maneuver_path,
                profile.participant_class,
                &mut work,
            )? {
                stream_count = stream_count.saturating_add(1);
                check_rows(gate_count, stream_count)?;
            }
        }
    }
    let rows_bytes = gate_count
        .saturating_mul(size_of::<GateCell>() as u64)
        .saturating_add(stream_count.saturating_mul(size_of::<StreamCell>() as u64));
    let base_bytes = fixed_bytes.saturating_add(rows_bytes);
    let base_records = fixed_records
        .saturating_add(gate_count)
        .saturating_add(stream_count);
    budget(unit, mir, base_bytes, base_records)?;
    let mut coverage = Vec::with_capacity(mir.conflict_passages.len());
    for stream in &mir.participant_streams {
        for passage in &mir.conflict_passages[stream.passages.as_usize_range()] {
            coverage.push(Coverage {
                gate: passage.admission_gate,
                zone: passage.conflict_zone,
            });
        }
    }
    coverage.sort_unstable_by_key(|v| (v.zone, v.gate));
    coverage.dedup_by_key(|v| (v.zone, v.gate));
    let mut protected_gates = vec![false; mir.maneuver_gates.len()];
    let mut minimum_stream_priority = vec![None::<i32>; mir.participant_streams.len()];
    let mut gates = Vec::with_capacity(gate_count as usize);
    let mut streams = Vec::with_capacity(stream_count as usize);
    for (g, gate) in mir.maneuver_gates.iter().enumerate() {
        for (v, profile) in mir.vehicle_profiles.iter().enumerate() {
            if access.path_allows(
                mir,
                gate.maneuver_path,
                profile.participant_class,
                &mut work,
            )? {
                gates.push(GateCell {
                    gate: MirManeuverGateKey::from_raw(g as u32),
                    profile: v as u32,
                    rule: 0,
                });
            }
        }
    }
    for (s, stream) in mir.participant_streams.iter().enumerate() {
        for (v, profile) in mir.vehicle_profiles.iter().enumerate() {
            if access.path_allows(
                mir,
                stream.maneuver_path,
                profile.participant_class,
                &mut work,
            )? {
                streams.push(StreamCell {
                    stream: MirParticipantStreamKey::from_raw(s as u32),
                    profile: v as u32,
                    rule: 0,
                });
            }
        }
    }
    let mut peak = base_bytes.max(lamp_bytes);
    for (policy_index, p) in mir.policies.iter().enumerate() {
        // 无实际准入行时仍需清空逐门/流工作表；这部分也不能绕过工作上限。
        work.charge(1 + mir.maneuver_gates.len() as u64 + mir.participant_streams.len() as u64)?;
        let index_bytes = (p.value.gates.len() as u64)
            .saturating_add(p.value.streams.len() as u64)
            .saturating_mul(size_of::<RuleIndex>() as u64);
        let bytes = base_bytes.saturating_add(index_bytes);
        let records = base_records
            .saturating_add(p.value.gates.len() as u64)
            .saturating_add(p.value.streams.len() as u64);
        budget(unit, mir, bytes, records)?;
        peak = peak.max(bytes);
        let mut gate_rules: Vec<_> = p
            .value
            .gates
            .iter()
            .enumerate()
            .map(|(i, r)| RuleIndex {
                owner: r.gate.raw(),
                rule: i as u32,
            })
            .collect();
        gate_rules.sort_unstable_by_key(|r| (r.owner, r.rule));
        let mut stream_rules: Vec<_> = p
            .value
            .streams
            .iter()
            .enumerate()
            .map(|(i, r)| RuleIndex {
                owner: r.stream.raw(),
                rule: i as u32,
            })
            .collect();
        stream_rules.sort_unstable_by_key(|r| (r.owner, r.rule));
        for cell in &mut gates {
            let class = mir.vehicle_profiles[cell.profile as usize].participant_class;
            cell.rule = select(unit, p, owner_rules(&gate_rules, cell.gate.raw()), |i| {
                Ok(specificity(mir, &p.value.gates[i].classes, class, &mut work)?.map(|d| (d, 0)))
            })?;
        }
        for cell in &mut streams {
            let class = mir.vehicle_profiles[cell.profile as usize].participant_class;
            cell.rule = select(
                unit,
                p,
                owner_rules(&stream_rules, cell.stream.raw()),
                |i| {
                    Ok(
                        specificity(mir, &p.value.streams[i].classes, class, &mut work)?
                            .map(|d| (d, p.value.streams[i].priority)),
                    )
                },
            )?;
        }
        minimum_stream_priority.fill(None);
        for cell in &streams {
            let priority = p.value.streams[cell.rule as usize].priority;
            let slot = &mut minimum_stream_priority[cell.stream.index()];
            *slot = Some(slot.map_or(priority, |old| old.min(priority)));
        }
        let targets = super::targets::build(unit, mir, p, &passages, bytes, records, &mut work)?;
        peak = peak.max(bytes.saturating_add(targets.bytes));
        for cell in &streams {
            let rule = &p.value.streams[cell.rule as usize];
            let start = targets.ranges.partition_point(|r| r.rule < cell.rule);
            let end = targets.ranges.partition_point(|r| r.rule <= cell.rule);
            for range in &targets.ranges[start..end] {
                work.charge(1 + range.targets.as_usize_range().len() as u64)?;
                for target in &targets.cells[range.targets.as_usize_range()] {
                    debug_assert_eq!(
                        mir.conflict_passages[range.subject_passage as usize].conflict_zone,
                        mir.conflict_passages[target.passage as usize].conflict_zone
                    );
                    if minimum_stream_priority[target.stream.index()]
                        .is_some_and(|priority| priority <= rule.priority)
                    {
                        return Err(fail(unit, p, Some(&rule.key), V::YieldPriority));
                    }
                }
            }
        }
        protected_gates.fill(false);
        for cell in &gates {
            let r = &p.value.gates[cell.rule as usize];
            if matches!(
                r.interpretation,
                I::ProtectedGroup | I::DirectionalRightProtected
            ) && r.prohibition != P::Always
            {
                protected_gates[cell.gate.index()] = true;
            }
        }
        if !protected.coherent(
            mir,
            policy_index as u32,
            &coverage,
            &protected_gates,
            &mut work,
        )? {
            return Err(fail(unit, p, None, V::ProtectedConflict));
        }
    }
    mir.peak_controlled_live_bytes = mir.peak_controlled_live_bytes.max(
        unit.controlled_live_bytes
            .saturating_add(mir.controlled_live_bytes)
            .saturating_add(peak),
    );
    Ok(())
}

fn select(
    unit: &CompilationUnit,
    p: &MirPolicy,
    entries: &[RuleIndex],
    mut rank: impl FnMut(usize) -> Result<Option<(u64, i32)>, DiagnosticBundle>,
) -> Result<u32, DiagnosticBundle> {
    let mut best = None;
    let mut ambiguous = false;
    for entry in entries {
        if let Some(candidate) = rank(entry.rule as usize)? {
            match best {
                None => {
                    best = Some((candidate, entry.rule));
                    ambiguous = false;
                }
                Some((current, _)) if candidate > current => {
                    best = Some((candidate, entry.rule));
                    ambiguous = false;
                }
                Some((current, _)) if candidate == current => ambiguous = true,
                _ => {}
            }
        }
    }
    if ambiguous {
        return Err(fail(unit, p, None, V::AmbiguousRule));
    }
    best.map(|(_, rule)| rule)
        .ok_or_else(|| fail(unit, p, None, V::MissingRule))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_checks_its_own_scratch_before_allocating_and_can_retry() {
        let (mut unit, mut mir) = super::super::fixture();
        let limits = unit.limits.clone();
        unit.limits = limits
            .clone()
            .with_test_admission_limit(CompileLimitDimension::StageScratchBytes, 1);
        let errors = validate(&unit, &mut mir).unwrap_err();
        assert!(errors.diagnostics().iter().any(|d| matches!(
            d.payload(),
            crate::DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::StageScratchBytes,
                ..
            }
        )));
        unit.limits = limits;
        validate(&unit, &mut mir).unwrap();
    }
}
