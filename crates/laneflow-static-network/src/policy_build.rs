//! 从受检 wire 独立闭合策略，不依赖编译器，也不借用输入 backing 发布。
mod resolve;

use crate::{
    BuildError, BuildStructure, PolicyBuildViolation as V, RangeU32, SharedConflictNetwork,
    SharedIdentityIndex, SharedNetworkBuildOptions, SharedPolicyNetwork, SharedTrafficNetwork,
    builder::{
        allocate_vec, checked_field, checked_ordinal_vector, checked_record_vector, checked_u8,
        checked_u32,
    },
    policy::*,
};
use core::mem::size_of;
use laneflow_format::{
    RegistryCheckedFieldValue as Value, RegistryCheckedRowView as Row,
    RegistryCheckedTableView as Table, ValueCheckedObjectView,
};
use laneflow_static_contract::{
    EntityKind, GateInterpretation, GateProhibition, ManeuverDirection, ManeuverGateOrdinal,
    MovementOrdinal, PolicyLocalMemberKind, RightOfWayPolicySetOrdinal,
};

const S: BuildStructure = BuildStructure::Policy;
const INVALID: BuildError = BuildError::InputInvariant { structure: S };
const OVERFLOW: BuildError = BuildError::ArithmeticOverflow { structure: S };

fn fail(policy: u32, violation: V) -> BuildError {
    BuildError::Policy { policy, violation }
}
fn raw(n: usize) -> Result<u32, BuildError> {
    u32::try_from(n).map_err(|_| OVERFLOW)
}
fn range(start: usize, end: usize) -> Result<RangeU32, BuildError> {
    Ok(RangeU32::new(
        raw(start)?,
        raw(end.checked_sub(start).ok_or(OVERFLOW)?)?,
    ))
}
fn table(
    view: ValueCheckedObjectView<'_>,
    section: u32,
    table: u32,
) -> Result<Table<'_>, BuildError> {
    view.registry_view()
        .section(section)
        .and_then(|s| s.table(table))
        .ok_or(INVALID)
}
fn text(row: Row<'_>, tag: u16) -> Result<&str, BuildError> {
    match checked_field(row, tag, S)? {
        Value::Utf8(v) => Ok(v),
        _ => Err(INVALID),
    }
}
fn optional_text(row: Row<'_>, tag: u16) -> Result<Option<&str>, BuildError> {
    row.field_by_tag(tag).map(|_| text(row, tag)).transpose()
}
fn number(row: Row<'_>, tag: u16) -> Result<u64, BuildError> {
    match checked_field(row, tag, S)? {
        Value::U64(v) => Ok(v),
        _ => Err(INVALID),
    }
}

struct Budget<'a> {
    options: SharedNetworkBuildOptions<'a>,
    retained: u64,
    scratch: u64,
    work: u64,
}
impl Budget<'_> {
    fn charge_work(&mut self, n: u64) -> Result<(), BuildError> {
        crate::builder::check_cancelled(self.options)?;
        self.work = self.work.checked_add(n).ok_or(OVERFLOW)?;
        Self::check(
            self.work,
            self.options.limits().max_policy_work(),
            BuildStructure::PolicyWork,
        )
    }
    fn check(required: u64, limit: u64, structure: BuildStructure) -> Result<(), BuildError> {
        if required > limit {
            Err(BuildError::BudgetExceeded {
                structure,
                required,
                limit,
            })
        } else {
            Ok(())
        }
    }
    fn retained(&mut self, bytes: u64) -> Result<(), BuildError> {
        self.retained = self.retained.checked_add(bytes).ok_or(OVERFLOW)?;
        Self::check(
            self.retained,
            self.options.limits().max_retained_bytes(),
            BuildStructure::RetainedOutput,
        )
    }
    fn allocate<T>(&mut self, n: u64, scratch: bool) -> Result<Vec<T>, BuildError> {
        let count = u32::try_from(n).map_err(|_| OVERFLOW)?;
        let bytes = n.checked_mul(size_of::<T>() as u64).ok_or(OVERFLOW)?;
        if scratch {
            self.scratch = self.scratch.checked_add(bytes).ok_or(OVERFLOW)?;
            Self::check(
                self.scratch,
                self.options.limits().max_scratch_bytes(),
                BuildStructure::BuilderScratch,
            )?;
        } else {
            self.retained(bytes)?;
        }
        allocate_vec(
            count,
            if scratch {
                BuildStructure::BuilderScratch
            } else {
                S
            },
        )
    }
    fn copy(&mut self, text: &str) -> Result<Box<str>, BuildError> {
        self.retained(text.len() as u64)?;
        let mut output = String::new();
        output
            .try_reserve_exact(text.len())
            .map_err(|_| BuildError::AllocationFailure { structure: S })?;
        output.push_str(text);
        Ok(output.into_boxed_str())
    }
    fn optional_copy(&mut self, text: Option<&str>) -> Result<Option<Box<str>>, BuildError> {
        text.map(|s| self.copy(s)).transpose()
    }
}

#[derive(Clone, Copy)]
struct Local<'a> {
    row: Row<'a>,
    policy: u32,
    key: &'a str,
}

fn locals<'a>(
    table: Table<'a>,
    policies: u32,
    budget: &mut Budget<'_>,
) -> Result<Vec<Local<'a>>, BuildError> {
    let mut values = budget.allocate(table.row_count().into(), true)?;
    let mut previous = None;
    for row in table.rows() {
        budget.charge_work(1)?;
        let policy = checked_u32(row, 1, S)?;
        let key = text(row, 2)?;
        if policy >= policies {
            return Err(fail(policy, V::Reference));
        }
        if previous.is_some_and(|old| old >= (policy, key)) {
            return Err(fail(policy, V::CanonicalMembers));
        }
        previous = Some((policy, key));
        values.push(Local { row, policy, key });
    }
    Ok(values)
}
fn lookup(values: &[Local<'_>], policy: u32, key: &str) -> Result<u32, BuildError> {
    values
        .binary_search_by(|v| (v.policy, v.key).cmp(&(policy, key)))
        .map_err(|_| fail(policy, V::Reference))
        .and_then(raw)
}
fn local_range(values: &[Local<'_>], policy: u32) -> Result<RangeU32, BuildError> {
    range(
        values.partition_point(|v| v.policy < policy),
        values.partition_point(|v| v.policy <= policy),
    )
}

pub(crate) fn build(
    view: ValueCheckedObjectView<'_>,
    traffic: &SharedTrafficNetwork,
    conflict: &SharedConflictNetwork,
    identity: &SharedIdentityIndex,
    prior_retained: u64,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<SharedPolicyNetwork, BuildError> {
    let mut budget = Budget {
        options,
        retained: prior_retained,
        scratch: 0,
        work: 0,
    };
    let policy_table = crate::relations_build::entity_table(view, EntityKind::RightOfWayPolicySet)?;
    let count = identity.entity_count(EntityKind::RightOfWayPolicySet);
    let evidence = locals(table(view, 3, 1)?, count, &mut budget)?;
    let gaps = locals(table(view, 3, 2)?, count, &mut budget)?;
    let streams = locals(table(view, 3, 3)?, count, &mut budget)?;
    let gates = locals(table(view, 3, 4)?, count, &mut budget)?;
    // 无策略时局部表已由 locals 验证为空，方向值域已由受检输入保证。
    // 不为旧的无策略路网分配任何策略查询工作表。
    if count == 0 {
        return Ok(SharedPolicyNetwork::empty());
    }
    let mut policies = budget.allocate(count.into(), false)?;
    // 既有 Access 闭合已验证所有已声明法规一致；本层再与每份策略比较。
    let mut regulation = None;
    for row in crate::relations_build::entity_table(view, EntityKind::AccessRule)?.rows() {
        budget.charge_work(1)?;
        if row.field_by_tag(7).is_some() {
            let records = checked_record_vector(row, 7, S)?;
            // regulation 是 optional singleton 的 wire record-vector；值域检查及
            // 先完成的 Access 关系闭合都已拒绝 len != 1，不存在被忽略的后续法规。
            let r = records.rows().next().ok_or(INVALID)?;
            let value = (text(r, 1)?, text(r, 2)?);
            if regulation.is_some_and(|old| old != value) {
                return Err(INVALID);
            }
            regulation = Some(value);
        }
    }
    for (i, row) in policy_table.rows().enumerate() {
        budget.charge_work(1)?;
        let p = raw(i)?;
        if checked_u32(row, 1, S)? != p {
            return Err(INVALID);
        }
        let jurisdiction = text(row, 3)?;
        let version = text(row, 4)?;
        if regulation.is_some_and(|r| r != (jurisdiction, version)) {
            return Err(fail(p, V::RegulationMismatch));
        }
        policies.push(PolicyRecord {
            id: identity
                .stable_id(RightOfWayPolicySetOrdinal::from_raw(p))
                .ok_or(INVALID)?,
            jurisdiction: budget.copy(jurisdiction)?,
            version: budget.copy(version)?,
            source: budget.optional_copy(optional_text(row, 5)?)?,
            gates: RangeU32::new(0, 0),
            streams: RangeU32::new(0, 0),
            gaps: local_range(&gaps, p)?,
        });
    }
    let mut evidence_records = budget.allocate(evidence.len() as u64, false)?;
    for e in &evidence {
        budget.charge_work(1)?;
        evidence_records.push(PolicyEvidence {
            key: budget.copy(e.key)?,
            locator: budget.copy(text(e.row, 3)?)?,
            description: budget.optional_copy(optional_text(e.row, 4)?)?,
        });
    }
    let mut gap_records = budget.allocate(gaps.len() as u64, false)?;
    for g in &gaps {
        budget.charge_work(1)?;
        gap_records.push(PolicyGapProfile {
            key: budget.copy(g.key)?,
            parameter_version: budget.copy(text(g.row, 3)?)?,
            minimum_lead_ms: number(g.row, 4)?,
            minimum_lag_ms: number(g.row, 5)?,
            clearance_ms: number(g.row, 6)?,
        });
    }
    let rule_count = (streams.len() as u64)
        .checked_add(gates.len() as u64)
        .ok_or(OVERFLOW)?;
    let mut rules = budget.allocate(rule_count, false)?;
    let mut evidence_count = 0_u64;
    for (members, tag) in [(&streams, 8), (&gates, 7)] {
        for r in members {
            budget.charge_work(1)?;
            evidence_count = evidence_count
                .checked_add(checked_record_vector(r.row, tag, S)?.len().into())
                .ok_or(OVERFLOW)?;
        }
    }
    let mut evidence_refs = budget.allocate(evidence_count, false)?;
    for (members, tag, kind) in [
        (&streams, 8, PolicyLocalMemberKind::StreamRule),
        (&gates, 7, PolicyLocalMemberKind::GateRule),
    ] {
        for r in members {
            let start = evidence_refs.len();
            let mut previous = None;
            for e in checked_record_vector(r.row, tag, S)?.rows() {
                budget.charge_work(1)?;
                let key = text(e, 1)?;
                if previous.is_some_and(|p| p >= key) {
                    return Err(fail(r.policy, V::Evidence));
                }
                previous = Some(key);
                evidence_refs.push(lookup(&evidence, r.policy, key)?);
            }
            if start == evidence_refs.len() && policies[r.policy as usize].source.is_none() {
                return Err(fail(r.policy, V::Evidence));
            }
            rules.push(RuleRecord {
                policy: RightOfWayPolicySetOrdinal::from_raw(r.policy),
                kind,
                key: budget.copy(r.key)?,
                evidence: range(start, evidence_refs.len())?,
            });
            if r.row.field_by_tag(4).is_some() {
                let selectors = checked_ordinal_vector(r.row, 4, S)?;
                if selectors.is_empty() {
                    return Err(fail(r.policy, V::Reference));
                }
                validate_ordinals(
                    ordinals(selectors),
                    identity.entity_count(EntityKind::ParticipantClass),
                    r.policy,
                    &mut budget,
                )?;
            }
        }
    }
    validate_gates(view, traffic, &gates, &mut budget)?;
    let resolved = resolve::build(
        traffic,
        conflict,
        &streams,
        &gates,
        &gaps,
        &mut policies,
        &mut budget,
    )?;
    Ok(SharedPolicyNetwork {
        policies: policies.into_boxed_slice(),
        rules: rules.into_boxed_slice(),
        evidence: evidence_records.into_boxed_slice(),
        evidence_refs: evidence_refs.into_boxed_slice(),
        gaps: gap_records.into_boxed_slice(),
        gate_owners: resolved.gate_owners.into_boxed_slice(),
        stream_owners: resolved.stream_owners.into_boxed_slice(),
        gates: resolved.gates.into_boxed_slice(),
        streams: resolved.streams.into_boxed_slice(),
        target_ranges: resolved.ranges.into_boxed_slice(),
        targets: resolved.targets.into_boxed_slice(),
    })
}

fn validate_ordinals(
    values: impl Iterator<Item = u32>,
    limit: u32,
    policy: u32,
    budget: &mut Budget<'_>,
) -> Result<(), BuildError> {
    let mut previous = None;
    for v in values {
        budget.charge_work(1)?;
        if v >= limit || previous.is_some_and(|p| p >= v) {
            return Err(fail(policy, V::Reference));
        }
        previous = Some(v);
    }
    Ok(())
}

fn validate_gates(
    view: ValueCheckedObjectView<'_>,
    traffic: &SharedTrafficNetwork,
    gates: &[Local<'_>],
    budget: &mut Budget<'_>,
) -> Result<(), BuildError> {
    let mut directions = budget.allocate(
        traffic.entity_counts().count(EntityKind::Movement).into(),
        true,
    )?;
    for row in table(view, 2, 5)?.rows() {
        budget.charge_work(1)?;
        let direction = row
            .field_by_tag(7)
            .map(|_| {
                checked_u8(row, 7, S)
                    .and_then(|code| ManeuverDirection::from_code(code).ok_or(INVALID))
            })
            .transpose()?;
        directions.push(direction);
    }
    let mut lamps = budget.allocate(
        traffic
            .entity_counts()
            .count(EntityKind::ManeuverGate)
            .into(),
        true,
    )?;
    lamps.resize(
        traffic.entity_counts().count(EntityKind::ManeuverGate) as usize,
        0_u8,
    );
    for r in gates {
        budget.charge_work(1)?;
        let gate = ManeuverGateOrdinal::from_raw(checked_u32(r.row, 3, S)?);
        let v = traffic
            .relations()
            .maneuver_gate(gate)
            .ok_or_else(|| fail(r.policy, V::Reference))?;
        let interpretation =
            GateInterpretation::from_code(checked_u8(r.row, 5, S)?).ok_or(INVALID)?;
        let prohibition = GateProhibition::from_code(checked_u8(r.row, 6, S)?).ok_or(INVALID)?;
        if (interpretation == GateInterpretation::Uncontrolled) != v.signal_group().is_none()
            || (v.signal_group().is_none() && prohibition == GateProhibition::OnRed)
        {
            return Err(fail(r.policy, V::SignalBinding));
        }
        let lamp = match interpretation {
            GateInterpretation::CnCircularRightTurn => 1,
            GateInterpretation::DirectionalRightProtected
            | GateInterpretation::DirectionalRightPermissive => 2,
            _ => 0,
        };
        if lamp != 0 {
            let movement: MovementOrdinal = traffic
                .maneuvers()
                .maneuver_path(v.path())
                .ok_or(INVALID)?
                .movement();
            if directions[movement.index()] != Some(ManeuverDirection::Right) {
                return Err(fail(r.policy, V::RightTurnRequired));
            }
            if lamps[gate.index()] != 0 && lamps[gate.index()] != lamp {
                return Err(fail(r.policy, V::LampTypeConflict));
            }
            lamps[gate.index()] = lamp;
        }
    }
    Ok(())
}

fn ordinals(v: laneflow_format::RegistryCheckedOrdinalVectorView<'_>) -> impl Iterator<Item = u32> {
    (0..v.len()).map(move |i| v.get(i).expect("checked ordinal vector"))
}
