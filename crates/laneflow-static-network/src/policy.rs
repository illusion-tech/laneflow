//! 世界无关的路权解析表。局部下标只在保留本根时有效。
use laneflow_static_contract::{
    ConflictZoneOrdinal, GateInterpretation, GateProhibition, ManeuverGateOrdinal,
    ParticipantStreamOrdinal, PolicyLocalMemberKind, RightOfWayPolicySetId,
    RightOfWayPolicySetOrdinal, VehicleProfileOrdinal,
};

use crate::RangeU32;

/// 可跨世界比较的规则身份；raw key 不作大小写或 Unicode 规范化。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PolicyRuleAttribution<'a> {
    pub policy: RightOfWayPolicySetId,
    pub member_kind: PolicyLocalMemberKind,
    pub key: &'a str,
}

pub(crate) struct PolicyRecord {
    pub(crate) id: RightOfWayPolicySetId,
    pub(crate) jurisdiction: Box<str>,
    pub(crate) version: Box<str>,
    pub(crate) source: Option<Box<str>>,
    pub(crate) gates: RangeU32,
    pub(crate) streams: RangeU32,
    pub(crate) gaps: RangeU32,
}

pub(crate) struct RuleRecord {
    pub(crate) policy: RightOfWayPolicySetOrdinal,
    pub(crate) kind: PolicyLocalMemberKind,
    pub(crate) key: Box<str>,
    pub(crate) evidence: RangeU32,
}

/// 一份规则的可追溯依据；冷数据由共享根拥有。
pub struct PolicyEvidence {
    pub(crate) key: Box<str>,
    pub(crate) locator: Box<str>,
    pub(crate) description: Option<Box<str>>,
}

impl PolicyEvidence {
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }
    #[must_use]
    pub fn locator(&self) -> &str {
        &self.locator
    }
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }
}

/// 编制的间隙参数；不包含任何世界步长派生值。
pub struct PolicyGapProfile {
    pub(crate) key: Box<str>,
    pub(crate) parameter_version: Box<str>,
    pub(crate) minimum_lead_ms: u64,
    pub(crate) minimum_lag_ms: u64,
    pub(crate) clearance_ms: u64,
}

impl PolicyGapProfile {
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }
    #[must_use]
    pub fn parameter_version(&self) -> &str {
        &self.parameter_version
    }
    #[must_use]
    pub const fn minimum_lead_ms(&self) -> u64 {
        self.minimum_lead_ms
    }
    #[must_use]
    pub const fn minimum_lag_ms(&self) -> u64 {
        self.minimum_lag_ms
    }
    #[must_use]
    pub const fn clearance_ms(&self) -> u64 {
        self.clearance_ms
    }
}

#[derive(Clone, Copy)]
pub(crate) struct PolicyOwner {
    pub(crate) owner: u32,
    pub(crate) cells: RangeU32,
}

/// 实际 Access 准入车型的门规则；不产生最终通行授权。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolvedGatePolicy {
    pub(crate) profile: VehicleProfileOrdinal,
    pub(crate) rule: u32,
    pub(crate) interpretation: GateInterpretation,
    pub(crate) prohibition: GateProhibition,
}

impl ResolvedGatePolicy {
    #[must_use]
    pub const fn profile(self) -> VehicleProfileOrdinal {
        self.profile
    }
    #[must_use]
    pub const fn interpretation(self) -> GateInterpretation {
        self.interpretation
    }
    #[must_use]
    pub const fn prohibition(self) -> GateProhibition {
        self.prohibition
    }
}

/// 实际 Access 准入车型的流规则。保留逐流 priority，覆盖最小值由仲裁候选求取。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolvedStreamPolicy {
    pub(crate) profile: VehicleProfileOrdinal,
    pub(crate) rule: u32,
    pub(crate) priority: i32,
    pub(crate) gap: Option<u32>,
    pub(crate) target_ranges: RangeU32,
}

impl ResolvedStreamPolicy {
    #[must_use]
    pub const fn profile(self) -> VehicleProfileOrdinal {
        self.profile
    }
    #[must_use]
    pub const fn priority(self) -> i32 {
        self.priority
    }
    /// 此策略内部的间隙参数下标，可用于世界自己的派生表。
    #[must_use]
    pub const fn gap_profile_index(self) -> Option<u32> {
        self.gap
    }
}

/// 一个 subject passage 的精确让行目标；未共享的 zone 不生成目标。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct YieldTargetCell {
    pub(crate) stream: ParticipantStreamOrdinal,
    pub(crate) passage_local_index: u32,
}
impl YieldTargetCell {
    #[must_use]
    pub const fn stream(self) -> ParticipantStreamOrdinal {
        self.stream
    }
    #[must_use]
    pub const fn passage_local_index(self) -> u32 {
        self.passage_local_index
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TargetRange {
    pub(crate) zone: ConflictZoneOrdinal,
    pub(crate) targets: RangeU32,
}

/// 策略 → 实际 owner → 实际 profile 的连续 CSR。唯一所有者是共享根。
pub struct SharedPolicyNetwork {
    pub(crate) policies: Box<[PolicyRecord]>,
    pub(crate) rules: Box<[RuleRecord]>,
    pub(crate) evidence: Box<[PolicyEvidence]>,
    pub(crate) evidence_refs: Box<[u32]>,
    pub(crate) gaps: Box<[PolicyGapProfile]>,
    pub(crate) gate_owners: Box<[PolicyOwner]>,
    pub(crate) stream_owners: Box<[PolicyOwner]>,
    pub(crate) gates: Box<[ResolvedGatePolicy]>,
    pub(crate) streams: Box<[ResolvedStreamPolicy]>,
    pub(crate) target_ranges: Box<[TargetRange]>,
    pub(crate) targets: Box<[YieldTargetCell]>,
}

/// 绑定共享根生命周期的单策略视图。
#[derive(Clone, Copy)]
pub struct PolicyView<'a> {
    pub(crate) network: &'a SharedPolicyNetwork,
    pub(crate) record: &'a PolicyRecord,
}

impl SharedPolicyNetwork {
    pub(crate) fn empty() -> Self {
        Self {
            policies: Box::new([]),
            rules: Box::new([]),
            evidence: Box::new([]),
            evidence_refs: Box::new([]),
            gaps: Box::new([]),
            gate_owners: Box::new([]),
            stream_owners: Box::new([]),
            gates: Box::new([]),
            streams: Box::new([]),
            target_ranges: Box::new([]),
            targets: Box::new([]),
        }
    }

    #[must_use]
    pub fn policy(&self, policy: RightOfWayPolicySetOrdinal) -> Option<PolicyView<'_>> {
        Some(PolicyView {
            network: self,
            record: self.policies.get(policy.index())?,
        })
    }

    #[must_use]
    pub fn retained_logical_bytes(&self) -> u64 {
        fn bytes<T>(v: &[T]) -> u64 {
            core::mem::size_of_val(v) as u64
        }
        fn optional(v: &Option<Box<str>>) -> u64 {
            v.as_ref().map_or(0, |s| s.len() as u64)
        }
        bytes(&self.policies)
            + bytes(&self.rules)
            + bytes(&self.evidence)
            + bytes(&self.evidence_refs)
            + bytes(&self.gaps)
            + bytes(&self.gate_owners)
            + bytes(&self.stream_owners)
            + bytes(&self.gates)
            + bytes(&self.streams)
            + bytes(&self.target_ranges)
            + bytes(&self.targets)
            + self
                .policies
                .iter()
                .map(|p| p.jurisdiction.len() as u64 + p.version.len() as u64 + optional(&p.source))
                .sum::<u64>()
            + self.rules.iter().map(|r| r.key.len() as u64).sum::<u64>()
            + self
                .evidence
                .iter()
                .map(|e| e.key.len() as u64 + e.locator.len() as u64 + optional(&e.description))
                .sum::<u64>()
            + self
                .gaps
                .iter()
                .map(|g| g.key.len() as u64 + g.parameter_version.len() as u64)
                .sum::<u64>()
    }
}

impl<'a> PolicyView<'a> {
    #[must_use]
    pub const fn id(self) -> RightOfWayPolicySetId {
        self.record.id
    }
    #[must_use]
    pub fn jurisdiction(self) -> &'a str {
        &self.record.jurisdiction
    }
    #[must_use]
    pub fn regulation_version(self) -> &'a str {
        &self.record.version
    }
    #[must_use]
    pub fn source(self) -> Option<&'a str> {
        self.record.source.as_deref()
    }
    #[must_use]
    pub fn gap_profiles(self) -> &'a [PolicyGapProfile] {
        self.record.gaps.slice(&self.network.gaps)
    }

    #[must_use]
    pub fn gate_profiles(self, gate: ManeuverGateOrdinal) -> &'a [ResolvedGatePolicy] {
        owner_cells(
            self.record.gates.slice(&self.network.gate_owners),
            gate.raw(),
            &self.network.gates,
        )
    }
    #[must_use]
    pub fn stream_profiles(self, stream: ParticipantStreamOrdinal) -> &'a [ResolvedStreamPolicy] {
        owner_cells(
            self.record.streams.slice(&self.network.stream_owners),
            stream.raw(),
            &self.network.streams,
        )
    }
    #[must_use]
    pub fn gate(
        self,
        gate: ManeuverGateOrdinal,
        profile: VehicleProfileOrdinal,
    ) -> Option<&'a ResolvedGatePolicy> {
        let cells = self.gate_profiles(gate);
        cells
            .binary_search_by_key(&profile, |c| c.profile)
            .ok()
            .map(|i| &cells[i])
    }
    #[must_use]
    pub fn stream(
        self,
        stream: ParticipantStreamOrdinal,
        profile: VehicleProfileOrdinal,
    ) -> Option<&'a ResolvedStreamPolicy> {
        let cells = self.stream_profiles(stream);
        cells
            .binary_search_by_key(&profile, |c| c.profile)
            .ok()
            .map(|i| &cells[i])
    }
    fn attribution(self, rule: u32) -> PolicyRuleAttribution<'a> {
        let r = &self.network.rules[rule as usize];
        PolicyRuleAttribution {
            policy: self.network.policies[r.policy.index()].id,
            member_kind: r.kind,
            key: &r.key,
        }
    }
    #[must_use]
    pub fn gate_attribution(
        self,
        gate: ManeuverGateOrdinal,
        profile: VehicleProfileOrdinal,
    ) -> Option<PolicyRuleAttribution<'a>> {
        Some(self.attribution(self.gate(gate, profile)?.rule))
    }
    #[must_use]
    pub fn stream_attribution(
        self,
        stream: ParticipantStreamOrdinal,
        profile: VehicleProfileOrdinal,
    ) -> Option<PolicyRuleAttribution<'a>> {
        Some(self.attribution(self.stream(stream, profile)?.rule))
    }
    /// 用 owner/profile 查询，避免把另一共享根的解析 cell 混入此根。
    #[must_use]
    pub fn yield_targets(
        self,
        stream: ParticipantStreamOrdinal,
        profile: VehicleProfileOrdinal,
        passage_local_index: u32,
    ) -> Option<(ConflictZoneOrdinal, &'a [YieldTargetCell])> {
        let cell = self.stream(stream, profile)?;
        let range = cell
            .target_ranges
            .slice(&self.network.target_ranges)
            .get(passage_local_index as usize)?;
        Some((range.zone, range.targets.slice(&self.network.targets)))
    }
    pub fn gate_evidence(
        self,
        gate: ManeuverGateOrdinal,
        profile: VehicleProfileOrdinal,
    ) -> Option<impl Iterator<Item = &'a PolicyEvidence>> {
        let rule = self.gate(gate, profile)?.rule;
        Some(self.rule_evidence(rule))
    }
    pub fn stream_evidence(
        self,
        stream: ParticipantStreamOrdinal,
        profile: VehicleProfileOrdinal,
    ) -> Option<impl Iterator<Item = &'a PolicyEvidence>> {
        let rule = self.stream(stream, profile)?.rule;
        Some(self.rule_evidence(rule))
    }
    fn rule_evidence(self, rule: u32) -> impl Iterator<Item = &'a PolicyEvidence> {
        self.network.rules[rule as usize]
            .evidence
            .slice(&self.network.evidence_refs)
            .iter()
            .map(|i| &self.network.evidence[*i as usize])
    }
}

fn owner_cells<'a, T>(owners: &[PolicyOwner], owner: u32, cells: &'a [T]) -> &'a [T] {
    owners
        .binary_search_by_key(&owner, |o| o.owner)
        .ok()
        .map_or(&[], |i| owners[i].cells.slice(cells))
}
