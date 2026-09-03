use super::*;
use laneflow_static_contract::{
    AccessEffect, ConflictZoneOrdinal, ManeuverPathOrdinal, ParticipantClassOrdinal,
    ParticipantStreamOrdinal, SignalAspect, SignalPhaseOrdinal, VehicleProfileOrdinal,
};

pub(super) struct Resolved {
    pub(super) gate_owners: Vec<PolicyOwner>,
    pub(super) stream_owners: Vec<PolicyOwner>,
    pub(super) gates: Vec<ResolvedGatePolicy>,
    pub(super) streams: Vec<ResolvedStreamPolicy>,
    pub(super) ranges: Vec<TargetRange>,
    pub(super) targets: Vec<YieldTargetCell>,
}
#[derive(Clone, Copy)]
struct Allowed {
    owner: u32,
    profile: VehicleProfileOrdinal,
}
#[derive(Clone, Copy)]
struct RuleIndex {
    policy: u32,
    owner: u32,
    rule: u32,
}
#[derive(Clone, Copy)]
struct Passage {
    stream: ParticipantStreamOrdinal,
    zone: ConflictZoneOrdinal,
    local: u32,
}
#[derive(Clone, Copy)]
struct Coverage {
    zone: ConflictZoneOrdinal,
    gate: ManeuverGateOrdinal,
}

fn allowed(
    traffic: &SharedTrafficNetwork,
    path: ManeuverPathOrdinal,
    profile: VehicleProfileOrdinal,
    budget: &mut Budget<'_>,
) -> Result<bool, BuildError> {
    budget.charge_work(1)?;
    let relations = traffic.relations();
    let class = relations.vehicle_profile(profile).ok_or(INVALID)?.class();
    let deny = |cell| {
        matches!(
            cell,
            Some(crate::AccessCell::Decided {
                effect: AccessEffect::Deny,
                ..
            })
        )
    };
    if deny(relations.path_access(path, class)) {
        return Ok(false);
    }
    for edge in traffic
        .maneuvers()
        .maneuver_path(path)
        .ok_or(INVALID)?
        .edges()
    {
        budget.charge_work(1)?;
        if deny(relations.edge_access(*edge, class)) {
            return Ok(false);
        }
    }
    Ok(true)
}
fn visit_allowed(
    traffic: &SharedTrafficNetwork,
    conflict: &SharedConflictNetwork,
    gate: bool,
    budget: &mut Budget<'_>,
    mut emit: impl FnMut(Allowed) -> Result<(), BuildError>,
) -> Result<(), BuildError> {
    let owners = traffic.entity_counts().count(if gate {
        EntityKind::ManeuverGate
    } else {
        EntityKind::ParticipantStream
    });
    for owner in 0..owners {
        budget.charge_work(1)?;
        let path = if gate {
            traffic
                .relations()
                .maneuver_gate(ManeuverGateOrdinal::from_raw(owner))
                .ok_or(INVALID)?
                .path()
        } else {
            conflict
                .participant_stream(ParticipantStreamOrdinal::from_raw(owner))
                .ok_or(INVALID)?
                .maneuver_path()
        };
        for p in 0..traffic.entity_counts().count(EntityKind::VehicleProfile) {
            let profile = VehicleProfileOrdinal::from_raw(p);
            if allowed(traffic, path, profile, budget)? {
                emit(Allowed { owner, profile })?;
            }
        }
    }
    Ok(())
}
fn access_rows(
    traffic: &SharedTrafficNetwork,
    conflict: &SharedConflictNetwork,
    gate: bool,
    budget: &mut Budget<'_>,
) -> Result<Vec<Allowed>, BuildError> {
    let mut count = 0_u64;
    visit_allowed(traffic, conflict, gate, budget, |_| {
        count = count.checked_add(1).ok_or(OVERFLOW)?;
        Ok(())
    })?;
    let mut rows = budget.allocate(count, true)?;
    visit_allowed(traffic, conflict, gate, budget, |v| {
        rows.push(v);
        Ok(())
    })?;
    Ok(rows)
}
fn index(
    rules: &[Local<'_>],
    owner_limit: u32,
    budget: &mut Budget<'_>,
) -> Result<Vec<RuleIndex>, BuildError> {
    let mut values = budget.allocate(rules.len() as u64, true)?;
    for (i, r) in rules.iter().enumerate() {
        budget.charge_work(1)?;
        let owner = checked_u32(r.row, 3, S)?;
        if owner >= owner_limit {
            return Err(fail(r.policy, V::Reference));
        }
        values.push(RuleIndex {
            policy: r.policy,
            owner,
            rule: raw(i)?,
        });
    }
    values.sort_unstable_by_key(|r| (r.policy, r.owner, r.rule));
    Ok(values)
}
fn select(
    traffic: &SharedTrafficNetwork,
    rules: &[Local<'_>],
    index: &[RuleIndex],
    policy: u32,
    cell: Allowed,
    stream: bool,
    budget: &mut Budget<'_>,
) -> Result<u32, BuildError> {
    budget.charge_work(1)?;
    let class = traffic
        .relations()
        .vehicle_profile(cell.profile)
        .ok_or(INVALID)?
        .class();
    let class = traffic
        .relations()
        .participant_class(class)
        .ok_or(INVALID)?;
    let position = class.subtree_range().0;
    let start = index.partition_point(|r| (r.policy, r.owner) < (policy, cell.owner));
    let end = index.partition_point(|r| (r.policy, r.owner) <= (policy, cell.owner));
    let mut best = None;
    let mut ambiguous = false;
    for entry in &index[start..end] {
        budget.charge_work(1)?;
        let r = rules[entry.rule as usize];
        let mut depth = Some(0_u64);
        if r.row.field_by_tag(4).is_some() {
            depth = None;
            for selector in ordinals(checked_ordinal_vector(r.row, 4, S)?) {
                budget.charge_work(1)?;
                let ancestor = traffic
                    .relations()
                    .participant_class(ParticipantClassOrdinal::from_raw(selector))
                    .ok_or(INVALID)?;
                let (enter, exit) = ancestor.subtree_range();
                if enter <= position && position < exit {
                    depth = Some(depth.unwrap_or(0).max(u64::from(ancestor.depth()) + 1));
                }
            }
        }
        let Some(depth) = depth else {
            continue;
        };
        let priority = if stream {
            crate::builder::checked_i32(r.row, 5, S)?
        } else {
            0
        };
        let rank = (depth, priority);
        match best {
            None => {
                best = Some((rank, entry.rule));
                ambiguous = false;
            }
            Some((old, _)) if rank > old => {
                best = Some((rank, entry.rule));
                ambiguous = false;
            }
            Some((old, _)) if rank == old => ambiguous = true,
            _ => {}
        }
    }
    if ambiguous {
        return Err(fail(policy, V::AmbiguousRule));
    }
    best.map(|(_, r)| r)
        .ok_or_else(|| fail(policy, V::MissingRule))
}

fn passage_index(
    traffic: &SharedTrafficNetwork,
    conflict: &SharedConflictNetwork,
    budget: &mut Budget<'_>,
) -> Result<Vec<Passage>, BuildError> {
    let mut n = 0_u64;
    let count = traffic.entity_counts().count(EntityKind::ParticipantStream);
    for s in 0..count {
        budget.charge_work(1)?;
        n = n
            .checked_add(
                conflict
                    .participant_stream(ParticipantStreamOrdinal::from_raw(s))
                    .ok_or(INVALID)?
                    .passages()
                    .len() as u64,
            )
            .ok_or(OVERFLOW)?;
    }
    let mut values = budget.allocate(n, true)?;
    for s in 0..count {
        let stream = ParticipantStreamOrdinal::from_raw(s);
        for (i, p) in conflict
            .participant_stream(stream)
            .ok_or(INVALID)?
            .passages()
            .iter()
            .enumerate()
        {
            budget.charge_work(1)?;
            values.push(Passage {
                stream,
                zone: p.conflict_zone(),
                local: raw(i)?,
            });
        }
    }
    values.sort_unstable_by_key(|p| (p.stream, p.zone));
    Ok(values)
}
fn in_zone(
    passages: &[Passage],
    stream: ParticipantStreamOrdinal,
    zone: ConflictZoneOrdinal,
) -> Option<Passage> {
    passages
        .binary_search_by_key(&(stream, zone), |p| (p.stream, p.zone))
        .ok()
        .map(|i| passages[i])
}
fn visit_targets(
    rule: Local<'_>,
    conflict: &SharedConflictNetwork,
    passages: &[Passage],
    budget: &mut Budget<'_>,
    mut emit: impl FnMut(ConflictZoneOrdinal, Option<YieldTargetCell>) -> Result<(), BuildError>,
) -> Result<(), BuildError> {
    let stream = ParticipantStreamOrdinal::from_raw(checked_u32(rule.row, 3, S)?);
    for source in conflict
        .participant_stream(stream)
        .ok_or(INVALID)?
        .passages()
    {
        budget.charge_work(1)?;
        emit(source.conflict_zone(), None)?;
        for t in ordinals(checked_ordinal_vector(rule.row, 6, S)?) {
            budget.charge_work(1)?;
            if let Some(p) = in_zone(
                passages,
                ParticipantStreamOrdinal::from_raw(t),
                source.conflict_zone(),
            ) {
                emit(
                    p.zone,
                    Some(YieldTargetCell {
                        stream: p.stream,
                        passage_local_index: p.local,
                    }),
                )?;
            }
        }
    }
    Ok(())
}

pub(super) fn build(
    traffic: &SharedTrafficNetwork,
    conflict: &SharedConflictNetwork,
    stream_rules: &[Local<'_>],
    gate_rules: &[Local<'_>],
    gaps: &[Local<'_>],
    policies: &mut [PolicyRecord],
    budget: &mut Budget<'_>,
) -> Result<Resolved, BuildError> {
    let passages = passage_index(traffic, conflict, budget)?;
    let stream_index = index(
        stream_rules,
        traffic.entity_counts().count(EntityKind::ParticipantStream),
        budget,
    )?;
    let gate_index = index(
        gate_rules,
        traffic.entity_counts().count(EntityKind::ManeuverGate),
        budget,
    )?;
    let mut rule_gaps = budget.allocate(stream_rules.len() as u64, true)?;
    let mut range_count = 0_u64;
    let mut target_count = 0_u64;
    for r in stream_rules {
        budget.charge_work(1)?;
        let owner = checked_u32(r.row, 3, S)?;
        let targets = checked_ordinal_vector(r.row, 6, S)?;
        validate_ordinals(
            ordinals(targets),
            traffic.entity_counts().count(EntityKind::ParticipantStream),
            r.policy,
            budget,
        )?;
        let gap = optional_text(r.row, 7)?;
        if targets.is_empty() != gap.is_none() {
            return Err(fail(r.policy, V::GapBinding));
        }
        rule_gaps.push(gap.map(|key| lookup(gaps, r.policy, key)).transpose()?);
        for target in ordinals(targets) {
            budget.charge_work(1)?;
            if owner == target {
                return Err(fail(r.policy, V::SelfYield));
            }
            let mut shared = false;
            for source in conflict
                .participant_stream(ParticipantStreamOrdinal::from_raw(owner))
                .ok_or(INVALID)?
                .passages()
            {
                budget.charge_work(1)?;
                if in_zone(
                    &passages,
                    ParticipantStreamOrdinal::from_raw(target),
                    source.conflict_zone(),
                )
                .is_some()
                {
                    shared = true;
                    break;
                }
            }
            if !shared {
                return Err(fail(r.policy, V::DisjointYield));
            }
        }
        visit_targets(*r, conflict, &passages, budget, |_, target| {
            let count = if target.is_some() {
                &mut target_count
            } else {
                &mut range_count
            };
            *count = count.checked_add(1).ok_or(OVERFLOW)?;
            Ok(())
        })?;
    }
    let mut ranges: Vec<TargetRange> = budget.allocate(range_count, false)?;
    let mut targets = budget.allocate(target_count, false)?;
    let mut rule_ranges = budget.allocate(stream_rules.len() as u64, true)?;
    for r in stream_rules {
        let start = ranges.len();
        visit_targets(*r, conflict, &passages, budget, |zone, target| {
            if let Some(t) = target {
                targets.push(t);
            } else {
                if let Some(last) = ranges.last_mut() {
                    last.targets = range(last.targets.start() as usize, targets.len())?;
                }
                ranges.push(TargetRange {
                    zone,
                    targets: range(targets.len(), targets.len())?,
                });
            }
            Ok(())
        })?;
        rule_ranges.push(range(start, ranges.len())?);
    }
    if let Some(last) = ranges.last_mut() {
        last.targets = range(last.targets.start() as usize, targets.len())?;
    }
    // 空策略仍允许构建旧内容；安装必须通过 NotRequired 的严格结构检查。
    let gate_rows = if policies.is_empty() {
        Vec::new()
    } else {
        access_rows(traffic, conflict, true, budget)?
    };
    let stream_rows = if policies.is_empty() {
        Vec::new()
    } else {
        access_rows(traffic, conflict, false, budget)?
    };
    let owner_count = |rows: &[Allowed]| {
        rows.iter()
            .enumerate()
            .filter(|(i, r)| *i == 0 || rows[i - 1].owner != r.owner)
            .count() as u64
    };
    let multiplied = |n: u64| n.checked_mul(policies.len() as u64).ok_or(OVERFLOW);
    let mut result = Resolved {
        gate_owners: budget.allocate(multiplied(owner_count(&gate_rows))?, false)?,
        stream_owners: budget.allocate(multiplied(owner_count(&stream_rows))?, false)?,
        gates: budget.allocate(multiplied(gate_rows.len() as u64)?, false)?,
        streams: budget.allocate(multiplied(stream_rows.len() as u64)?, false)?,
        ranges,
        targets,
    };
    let mut protected = Protected::build(traffic, conflict, &passages, budget)?;
    let mut minimum = budget.allocate(
        traffic
            .entity_counts()
            .count(EntityKind::ParticipantStream)
            .into(),
        true,
    )?;
    minimum.resize(
        traffic.entity_counts().count(EntityKind::ParticipantStream) as usize,
        None::<i32>,
    );
    let mut protected_gates = budget.allocate(
        traffic
            .entity_counts()
            .count(EntityKind::ManeuverGate)
            .into(),
        true,
    )?;
    protected_gates.resize(
        traffic.entity_counts().count(EntityKind::ManeuverGate) as usize,
        false,
    );
    for (p, policy) in policies.iter_mut().enumerate() {
        let p = raw(p)?;
        budget.charge_work(1 + minimum.len() as u64 + protected_gates.len() as u64)?;
        minimum.fill(None);
        protected_gates.fill(false);
        let gs = result.gate_owners.len();
        for cell in &gate_rows {
            let rule = select(traffic, gate_rules, &gate_index, p, *cell, false, budget)?;
            let r = gate_rules[rule as usize];
            push_owner(&mut result.gate_owners, gs, cell.owner, result.gates.len())?;
            let interpretation =
                GateInterpretation::from_code(checked_u8(r.row, 5, S)?).ok_or(INVALID)?;
            let prohibition =
                GateProhibition::from_code(checked_u8(r.row, 6, S)?).ok_or(INVALID)?;
            protected_gates[cell.owner as usize] |= matches!(
                interpretation,
                GateInterpretation::ProtectedGroup | GateInterpretation::DirectionalRightProtected
            ) && prohibition != GateProhibition::Always;
            result.gates.push(ResolvedGatePolicy {
                profile: cell.profile,
                rule: raw(stream_rules.len())?.checked_add(rule).ok_or(OVERFLOW)?,
                interpretation,
                prohibition,
            });
        }
        finish_owner(&mut result.gate_owners, gs, result.gates.len())?;
        policy.gates = range(gs, result.gate_owners.len())?;
        let ss = result.stream_owners.len();
        let cells_start = result.streams.len();
        for cell in &stream_rows {
            let rule = select(traffic, stream_rules, &stream_index, p, *cell, true, budget)?;
            let priority = crate::builder::checked_i32(stream_rules[rule as usize].row, 5, S)?;
            let m = &mut minimum[cell.owner as usize];
            *m = Some(m.map_or(priority, |old| old.min(priority)));
            push_owner(
                &mut result.stream_owners,
                ss,
                cell.owner,
                result.streams.len(),
            )?;
            result.streams.push(ResolvedStreamPolicy {
                profile: cell.profile,
                rule,
                priority,
                gap: rule_gaps[rule as usize].map(|i| i - policy.gaps.start()),
                target_ranges: rule_ranges[rule as usize],
            });
        }
        finish_owner(&mut result.stream_owners, ss, result.streams.len())?;
        policy.streams = range(ss, result.stream_owners.len())?;
        for cell in &result.streams[cells_start..] {
            for r in cell.target_ranges.slice(&result.ranges) {
                budget.charge_work(1)?;
                for t in r.targets.slice(&result.targets) {
                    budget.charge_work(1)?;
                    if minimum[t.stream.index()].is_some_and(|m| m <= cell.priority) {
                        return Err(fail(p, V::YieldPriority));
                    }
                }
            }
        }
        protected.validate(traffic, p, &protected_gates, budget)?;
    }
    Ok(result)
}

fn push_owner(
    owners: &mut Vec<PolicyOwner>,
    policy_start: usize,
    owner: u32,
    cells: usize,
) -> Result<(), BuildError> {
    if owners.len() == policy_start || owners.last().is_none_or(|r| r.owner != owner) {
        finish_owner(owners, policy_start, cells)?;
        owners.push(PolicyOwner {
            owner,
            cells: range(cells, cells)?,
        });
    }
    Ok(())
}
fn finish_owner(
    owners: &mut [PolicyOwner],
    policy_start: usize,
    cells: usize,
) -> Result<(), BuildError> {
    if owners.len() > policy_start {
        let last = owners.last_mut().ok_or(INVALID)?;
        last.cells = range(last.cells.start() as usize, cells)?;
    }
    Ok(())
}

struct Protected {
    coverage: Vec<Coverage>,
    green: Vec<(laneflow_static_contract::SignalGroupOrdinal, u32)>,
    seen: Vec<Option<(u32, ConflictZoneOrdinal)>>,
}
impl Protected {
    fn build(
        traffic: &SharedTrafficNetwork,
        conflict: &SharedConflictNetwork,
        passages: &[Passage],
        budget: &mut Budget<'_>,
    ) -> Result<Self, BuildError> {
        let mut coverage = budget.allocate(passages.len() as u64, true)?;
        for p in passages {
            budget.charge_work(1)?;
            let passage = &conflict
                .participant_stream(p.stream)
                .ok_or(INVALID)?
                .passages()[p.local as usize];
            coverage.push(Coverage {
                zone: p.zone,
                gate: passage.admission_gate(),
            });
        }
        coverage.sort_unstable_by_key(|c| (c.zone, c.gate));
        coverage.dedup_by_key(|c| (c.zone, c.gate));
        let phase_count = traffic.entity_counts().count(EntityKind::SignalPhase);
        let mut count = 0_u64;
        for phase in 0..phase_count {
            budget.charge_work(1)?;
            for (_, aspect) in traffic
                .relations()
                .signal_phase(SignalPhaseOrdinal::from_raw(phase))
                .ok_or(INVALID)?
                .states()
            {
                budget.charge_work(1)?;
                if aspect == SignalAspect::Green {
                    count = count.checked_add(1).ok_or(OVERFLOW)?;
                }
            }
        }
        let mut green = budget.allocate(count, true)?;
        for phase in 0..phase_count {
            for (group, aspect) in traffic
                .relations()
                .signal_phase(SignalPhaseOrdinal::from_raw(phase))
                .ok_or(INVALID)?
                .states()
            {
                budget.charge_work(1)?;
                if aspect == SignalAspect::Green {
                    green.push((group, phase));
                }
            }
        }
        green.sort_unstable();
        let mut seen = budget.allocate(phase_count.into(), true)?;
        seen.resize(phase_count as usize, None);
        Ok(Self {
            coverage,
            green,
            seen,
        })
    }
    fn validate(
        &mut self,
        traffic: &SharedTrafficNetwork,
        policy: u32,
        protected: &[bool],
        budget: &mut Budget<'_>,
    ) -> Result<(), BuildError> {
        let mut zone = None;
        let mut controller = None;
        for c in &self.coverage {
            budget.charge_work(1)?;
            if zone != Some(c.zone) {
                zone = Some(c.zone);
                controller = None;
            }
            if !protected[c.gate.index()] {
                continue;
            }
            let group = traffic
                .relations()
                .gate_signal_group(c.gate)
                .ok_or(INVALID)?;
            let start = self.green.partition_point(|(g, _)| *g < group);
            let end = self.green.partition_point(|(g, _)| *g <= group);
            if start == end {
                continue;
            }
            let current = traffic
                .relations()
                .signal_group(group)
                .ok_or(INVALID)?
                .controller();
            if controller.is_some_and(|old| old != current) {
                return Err(fail(policy, V::ProtectedConflict));
            }
            controller = Some(current);
            for (_, phase) in &self.green[start..end] {
                budget.charge_work(1)?;
                let stamp = Some((policy, c.zone));
                if self.seen[*phase as usize] == stamp {
                    return Err(fail(policy, V::ProtectedConflict));
                }
                self.seen[*phase as usize] = stamp;
            }
        }
        Ok(())
    }
}
