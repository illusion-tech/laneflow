//! 全部声明的合法性与实际 Access 准入行的静态解析。
use super::access::AccessIndex;
use super::*;
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
) -> Option<u64> {
    selectors.as_ref().map_or(Some(0), |v| {
        v.iter()
            .filter_map(|&a| class_depth(mir, a, class))
            .max()
            .map(|d| u64::from(d) + 1)
    })
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
    priority: Option<i32>,
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
struct Coverage {
    gate: MirManeuverGateKey,
    stream: MirParticipantStreamKey,
    zone: MirConflictZoneKey,
}
fn gate_coverage(entries: &[Coverage], gate: MirManeuverGateKey) -> &[Coverage] {
    &entries
        [entries.partition_point(|v| v.gate < gate)..entries.partition_point(|v| v.gate <= gate)]
}
fn owner_rules(entries: &[RuleIndex], owner: u32) -> &[RuleIndex] {
    &entries[entries.partition_point(|v| v.owner < owner)
        ..entries.partition_point(|v| v.owner <= owner)]
}
fn stream_rows(entries: &[StreamCell], owner: MirParticipantStreamKey) -> &[StreamCell] {
    &entries[entries.partition_point(|v| v.stream < owner)
        ..entries.partition_point(|v| v.stream <= owner)]
}
fn passages(mir: &MirUnit, stream: MirParticipantStreamKey) -> &[super::super::MirConflictPassage] {
    &mir.conflict_passages[mir.participant_streams[stream.index()]
        .passages
        .as_usize_range()]
}

pub(crate) fn validate(unit: &CompilationUnit, mir: &mut MirUnit) -> Result<(), DiagnosticBundle> {
    if mir.policies.is_empty() {
        return Ok(());
    }
    let lamp_bytes = mir.maneuver_gates.len() as u64;
    budget(unit, mir, lamp_bytes, 0)?;
    let mut lamps = vec![0_u8; mir.maneuver_gates.len()];
    for p in &mir.policies {
        for access in &mir.access_rules {
            if let Some(reg) = &access.regulation
                && (reg.jurisdiction != p.value.regulation.jurisdiction
                    || reg.version != p.value.regulation.version)
            {
                return Err(fail(unit, p, None, V::RegulationMismatch));
            }
        }
        for r in &p.value.gates {
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
            for &target in &r.yield_to {
                if target == r.stream {
                    return Err(fail(unit, p, Some(&r.key), V::SelfYield));
                }
                if !passages(mir, r.stream).iter().any(|a| {
                    passages(mir, target)
                        .iter()
                        .any(|b| a.conflict_zone == b.conflict_zone)
                }) {
                    return Err(fail(unit, p, Some(&r.key), V::DisjointYield));
                }
            }
        }
    }
    drop(lamps);
    let access = AccessIndex::build(unit, mir, 0)?;
    // 只计实际可进入行；不预分配 policy × gate/stream × profile 上界。
    let mut gate_count = 0_u64;
    let mut stream_count = 0_u64;
    let coverage_bytes = (mir.conflict_passages.len() as u64)
        .saturating_mul(2 * size_of::<Coverage>() as u64)
        .saturating_add(mir.maneuver_gates.len() as u64)
        .saturating_add(
            (mir.participant_streams.len() as u64).saturating_mul(size_of::<Option<i32>>() as u64),
        );
    let check_rows = |gates: u64, streams: u64| {
        budget(
            unit,
            mir,
            access
                .bytes()
                .saturating_add(coverage_bytes)
                .saturating_add(gates.saturating_mul(size_of::<GateCell>() as u64))
                .saturating_add(streams.saturating_mul(size_of::<StreamCell>() as u64)),
            gates
                .saturating_add(streams)
                .saturating_add(mir.conflict_passages.len() as u64),
        )
    };
    check_rows(0, 0)?;
    for gate in &mir.maneuver_gates {
        for profile in &mir.vehicle_profiles {
            if access.path_allows(mir, gate.maneuver_path, profile.participant_class) {
                gate_count = gate_count.saturating_add(1);
                check_rows(gate_count, stream_count)?;
            }
        }
    }
    for stream in &mir.participant_streams {
        for profile in &mir.vehicle_profiles {
            if access.path_allows(mir, stream.maneuver_path, profile.participant_class) {
                stream_count = stream_count.saturating_add(1);
                check_rows(gate_count, stream_count)?;
            }
        }
    }
    let rows_bytes = gate_count
        .saturating_mul(size_of::<GateCell>() as u64)
        .saturating_add(stream_count.saturating_mul(size_of::<StreamCell>() as u64));
    let base_bytes = access
        .bytes()
        .saturating_add(rows_bytes)
        .saturating_add(coverage_bytes);
    budget(
        unit,
        mir,
        base_bytes,
        gate_count
            .saturating_add(stream_count)
            .saturating_add(mir.conflict_passages.len() as u64),
    )?;
    let mut coverage = Vec::with_capacity(mir.conflict_passages.len());
    for (index, stream) in mir.participant_streams.iter().enumerate() {
        for passage in &mir.conflict_passages[stream.passages.as_usize_range()] {
            coverage.push(Coverage {
                gate: passage.admission_gate,
                stream: MirParticipantStreamKey::from_raw(index as u32),
                zone: passage.conflict_zone,
            });
        }
    }
    coverage.sort_unstable_by_key(|v| (v.gate, v.stream, v.zone));
    let mut zone_gates = coverage.clone();
    zone_gates.sort_unstable_by_key(|v| (v.zone, v.gate));
    zone_gates.dedup_by_key(|v| (v.zone, v.gate));
    let mut protected_gates = vec![false; mir.maneuver_gates.len()];
    let mut minimum_stream_priority = vec![None::<i32>; mir.participant_streams.len()];
    let mut gates = Vec::with_capacity(gate_count as usize);
    let mut streams = Vec::with_capacity(stream_count as usize);
    for (g, gate) in mir.maneuver_gates.iter().enumerate() {
        for (v, profile) in mir.vehicle_profiles.iter().enumerate() {
            if access.path_allows(mir, gate.maneuver_path, profile.participant_class) {
                gates.push(GateCell {
                    gate: MirManeuverGateKey::from_raw(g as u32),
                    profile: v as u32,
                    rule: 0,
                    priority: None,
                });
            }
        }
    }
    for (s, stream) in mir.participant_streams.iter().enumerate() {
        for (v, profile) in mir.vehicle_profiles.iter().enumerate() {
            if access.path_allows(mir, stream.maneuver_path, profile.participant_class) {
                streams.push(StreamCell {
                    stream: MirParticipantStreamKey::from_raw(s as u32),
                    profile: v as u32,
                    rule: 0,
                });
            }
        }
    }
    let mut peak = base_bytes.max(lamp_bytes);
    for p in &mir.policies {
        let index_bytes = (p.value.gates.len() as u64)
            .saturating_add(p.value.streams.len() as u64)
            .saturating_mul(size_of::<RuleIndex>() as u64);
        let bytes = base_bytes.saturating_add(index_bytes);
        budget(unit, mir, bytes, gate_count.saturating_add(stream_count))?;
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
            cell.rule = select(owner_rules(&gate_rules, cell.gate.raw()), |i| {
                specificity(mir, &p.value.gates[i].classes, class).map(|d| (d, 0))
            })
            .map_err(|v| fail(unit, p, None, v))?;
            cell.priority = None;
        }
        for cell in &mut streams {
            let class = mir.vehicle_profiles[cell.profile as usize].participant_class;
            cell.rule = select(owner_rules(&stream_rules, cell.stream.raw()), |i| {
                specificity(mir, &p.value.streams[i].classes, class)
                    .map(|d| (d, p.value.streams[i].priority))
            })
            .map_err(|v| fail(unit, p, None, v))?;
        }
        minimum_stream_priority.fill(None);
        for cell in &streams {
            let priority = p.value.streams[cell.rule as usize].priority;
            let slot = &mut minimum_stream_priority[cell.stream.index()];
            *slot = Some(slot.map_or(priority, |old| old.min(priority)));
        }
        let targets =
            super::targets::build(unit, mir, p, bytes, gate_count.saturating_add(stream_count))?;
        peak = peak.max(bytes.saturating_add(targets.bytes));
        for cell in &streams {
            let rule = &p.value.streams[cell.rule as usize];
            let start = targets.ranges.partition_point(|r| r.rule < cell.rule);
            let end = targets.ranges.partition_point(|r| r.rule <= cell.rule);
            for range in &targets.ranges[start..end] {
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
        // 每个 Gate coverage 对所有实际 subject stream 取 minimum；纯 Waiting 保留 None。
        for cell in &mut gates {
            cell.priority = coverage_priority(
                p,
                &streams,
                gate_coverage(&coverage, cell.gate),
                cell.profile,
            );
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
        protected_coherence(unit, mir, p, &zone_gates, &protected_gates)?;
    }
    mir.peak_controlled_live_bytes = mir.peak_controlled_live_bytes.max(
        unit.controlled_live_bytes
            .saturating_add(mir.controlled_live_bytes)
            .saturating_add(peak),
    );
    Ok(())
}

fn coverage_priority(
    p: &MirPolicy,
    streams: &[StreamCell],
    coverage: &[Coverage],
    profile: u32,
) -> Option<i32> {
    coverage
        .iter()
        .filter_map(|covered| {
            let rows = stream_rows(streams, covered.stream);
            rows.binary_search_by_key(&profile, |r| r.profile)
                .ok()
                .map(|index| p.value.streams[rows[index].rule as usize].priority)
        })
        .min()
}

fn select(entries: &[RuleIndex], rank: impl Fn(usize) -> Option<(u64, i32)>) -> Result<u32, V> {
    let mut best = None;
    let mut ambiguous = false;
    for entry in entries {
        if let Some(candidate) = rank(entry.rule as usize) {
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
        return Err(V::AmbiguousRule);
    }
    best.map(|(_, rule)| rule).ok_or(V::MissingRule)
}

fn protected_coherence(
    unit: &CompilationUnit,
    mir: &MirUnit,
    p: &MirPolicy,
    cells: &[Coverage],
    protected: &[bool],
) -> Result<(), DiagnosticBundle> {
    for (index, a) in cells
        .iter()
        .enumerate()
        .filter(|(_, c)| protected[c.gate.index()])
    {
        for b in cells[index + 1..]
            .iter()
            .take_while(|c| c.zone == a.zone)
            .filter(|c| protected[c.gate.index()])
        {
            let (
                MirSignalControl::Group {
                    signal_group: a_group,
                    ..
                },
                MirSignalControl::Group {
                    signal_group: b_group,
                    ..
                },
            ) = (
                &mir.maneuver_gates[a.gate.index()].signal_control,
                &mir.maneuver_gates[b.gate.index()].signal_control,
            )
            else {
                unreachable!("protected binding validated");
            };
            let a_controller = mir.signal_groups[a_group.index()].controller;
            let b_controller = mir.signal_groups[b_group.index()].controller;
            let green = |phase: &super::super::MirSignalPhase, group| {
                mir.signal_phase_states[phase.states.as_usize_range()]
                    .iter()
                    .any(|s| {
                        s.signal_group == group
                            && s.aspect == laneflow_static_contract::SignalAspect::Green
                    })
            };
            let conflict = if a_controller == b_controller {
                mir.signal_phases[mir.signal_controllers[a_controller.index()]
                    .phases
                    .as_usize_range()]
                .iter()
                .any(|phase| green(phase, *a_group) && green(phase, *b_group))
            } else {
                mir.signal_phases[mir.signal_controllers[a_controller.index()]
                    .phases
                    .as_usize_range()]
                .iter()
                .any(|phase| green(phase, *a_group))
                    && mir.signal_phases[mir.signal_controllers[b_controller.index()]
                        .phases
                        .as_usize_range()]
                    .iter()
                    .any(|phase| green(phase, *b_group))
            };
            if conflict {
                return Err(fail(unit, p, None, V::ProtectedConflict));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_uses_minimum_of_all_resolved_subjects_and_waiting_is_absent() {
        let (_, mir) = super::super::fixture();
        let p = &mir.policies[0];
        let gate = p.value.gates[0].gate;
        let mut rows: Vec<_> = p
            .value
            .streams
            .iter()
            .enumerate()
            .map(|(i, rule)| StreamCell {
                stream: rule.stream,
                profile: 0,
                rule: i as u32,
            })
            .collect();
        rows.sort_unstable_by_key(|r| (r.stream, r.profile));
        let coverage: Vec<_> = p
            .value
            .streams
            .iter()
            .map(|rule| Coverage {
                gate,
                stream: rule.stream,
                zone: MirConflictZoneKey::from_raw(0),
            })
            .collect();
        assert_eq!(coverage_priority(p, &rows, &coverage, 0), Some(1));
        let mut reversed = coverage.clone();
        reversed.reverse();
        reversed.push(reversed[0]);
        assert_eq!(coverage_priority(p, &rows, &reversed, 0), Some(1));
        assert_eq!(coverage_priority(p, &rows, &[], 0), None);
        assert_eq!(coverage_priority(p, &rows, &coverage, 1), None);
    }

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
