//! 世界无关的路权解析表。局部下标只在保留本根时有效。
use laneflow_static_contract::{
    ConflictZoneOrdinal, GateInterpretation, GateProhibition, ManeuverGateOrdinal,
    ParticipantClassOrdinal, ParticipantStreamOrdinal, PolicyLocalMemberKind,
    RightOfWayPolicySetId, RightOfWayPolicySetOrdinal,
};

use crate::RangeU32;

/// 可跨世界比较的规则身份；raw key 不作大小写或 Unicode 规范化。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PolicyRuleAttribution<'a> {
    pub policy: RightOfWayPolicySetId,
    pub member_kind: PolicyLocalMemberKind,
    pub key: &'a str,
}

/// 单份路权策略集的内部记录；区间字段指向共享根的连续 payload。
pub(crate) struct PolicyRecord {
    pub(crate) id: RightOfWayPolicySetId,
    pub(crate) jurisdiction: Box<str>,
    pub(crate) version: Box<str>,
    pub(crate) source: Option<Box<str>>,
    pub(crate) gates: RangeU32,
    pub(crate) streams: RangeU32,
    pub(crate) gaps: RangeU32,
}

/// 单条策略成员规则的内部记录。
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
    /// 依据的标识键。
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }
    /// 依据在来源中的定位值。
    #[must_use]
    pub fn locator(&self) -> &str {
        &self.locator
    }
    /// 依据的可选可读描述。
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
    /// 间隙参数的标识键。
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }
    /// 间隙参数的版本标识。
    #[must_use]
    pub fn parameter_version(&self) -> &str {
        &self.parameter_version
    }
    /// 最小领先间隙（毫秒）。
    #[must_use]
    pub const fn minimum_lead_ms(&self) -> u64 {
        self.minimum_lead_ms
    }
    /// 最小滞后间隙（毫秒）。
    #[must_use]
    pub const fn minimum_lag_ms(&self) -> u64 {
        self.minimum_lag_ms
    }
    /// 清空缓冲（毫秒）。
    #[must_use]
    pub const fn clearance_ms(&self) -> u64 {
        self.clearance_ms
    }
}

/// 规则 owner 到其连续 cell 区间的内部映射记录。
#[derive(Clone, Copy)]
pub(crate) struct PolicyOwner {
    pub(crate) owner: u32,
    pub(crate) cells: RangeU32,
}

/// 实际 Access 准入类别的门规则；不产生最终通行授权。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolvedGatePolicy {
    pub(crate) class: ParticipantClassOrdinal,
    pub(crate) rule: u32,
    pub(crate) interpretation: GateInterpretation,
    pub(crate) prohibition: GateProhibition,
}

impl ResolvedGatePolicy {
    /// 规则实际适用的参与者类别。
    #[must_use]
    pub const fn class(self) -> ParticipantClassOrdinal {
        self.class
    }
    /// 门规则的合规解释。
    #[must_use]
    pub const fn interpretation(self) -> GateInterpretation {
        self.interpretation
    }
    /// 门规则的禁令语义。
    #[must_use]
    pub const fn prohibition(self) -> GateProhibition {
        self.prohibition
    }
}

/// 实际 Access 准入类别的流规则。保留逐流 priority，覆盖最小值由仲裁候选求取。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolvedStreamPolicy {
    pub(crate) class: ParticipantClassOrdinal,
    pub(crate) rule: u32,
    pub(crate) priority: i32,
    pub(crate) gap: Option<u32>,
    pub(crate) target_ranges: RangeU32,
}

impl ResolvedStreamPolicy {
    /// 规则实际适用的参与者类别。
    #[must_use]
    pub const fn class(self) -> ParticipantClassOrdinal {
        self.class
    }
    /// 该流的法规优先级。
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
    /// 让行目标所属的参与者流。
    #[must_use]
    pub const fn stream(self) -> ParticipantStreamOrdinal {
        self.stream
    }
    /// 让行目标通行段在其所有者流内的局部下标。
    #[must_use]
    pub const fn passage_local_index(self) -> u32 {
        self.passage_local_index
    }
}

/// 冲突区到其让行目标区间的内部记录。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TargetRange {
    pub(crate) zone: ConflictZoneOrdinal,
    pub(crate) targets: RangeU32,
}

/// 策略 → 实际 owner → 实际 class 的连续 CSR。唯一所有者是共享根。
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
    /// 构造不含任何策略的空共享根。
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

    /// 按 ordinal 借用单份策略的视图。
    #[must_use]
    pub fn policy(&self, policy: RightOfWayPolicySetOrdinal) -> Option<PolicyView<'_>> {
        Some(PolicyView {
            network: self,
            record: self.policies.get(policy.index())?,
        })
    }

    /// 本共享根保留的逻辑字节数（含字符串冷数据）。
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
    /// 该路权策略集的稳定身份。
    #[must_use]
    pub const fn id(self) -> RightOfWayPolicySetId {
        self.record.id
    }
    /// 法规身份的法域。
    #[must_use]
    pub fn jurisdiction(self) -> &'a str {
        &self.record.jurisdiction
    }
    /// 法规身份的法规版本。
    #[must_use]
    pub fn regulation_version(self) -> &'a str {
        &self.record.version
    }
    /// 法规身份的可选来源。
    #[must_use]
    pub fn source(self) -> Option<&'a str> {
        self.record.source.as_deref()
    }
    /// 该策略集局部的间隙接受参数表。
    #[must_use]
    pub fn gap_profiles(self) -> &'a [PolicyGapProfile] {
        self.record.gaps.slice(&self.network.gaps)
    }

    /// 指定机动门在各参与者类别下解析出的门规则单元。
    #[must_use]
    pub fn gate_classes(self, gate: ManeuverGateOrdinal) -> &'a [ResolvedGatePolicy] {
        owner_cells(
            self.record.gates.slice(&self.network.gate_owners),
            gate.raw(),
            &self.network.gates,
        )
    }
    /// 指定参与者流在各参与者类别下解析出的流规则。
    #[must_use]
    pub fn stream_classes(self, stream: ParticipantStreamOrdinal) -> &'a [ResolvedStreamPolicy] {
        owner_cells(
            self.record.streams.slice(&self.network.stream_owners),
            stream.raw(),
            &self.network.streams,
        )
    }
    /// 按机动门与参与者类别查询唯一解析的门规则单元。
    #[must_use]
    pub fn gate(
        self,
        gate: ManeuverGateOrdinal,
        class: ParticipantClassOrdinal,
    ) -> Option<&'a ResolvedGatePolicy> {
        let cells = self.gate_classes(gate);
        cells
            .binary_search_by_key(&class, |c| c.class)
            .ok()
            .map(|i| &cells[i])
    }
    /// 按参与者流与参与者类别查询唯一解析的流规则。
    #[must_use]
    pub fn stream(
        self,
        stream: ParticipantStreamOrdinal,
        class: ParticipantClassOrdinal,
    ) -> Option<&'a ResolvedStreamPolicy> {
        let cells = self.stream_classes(stream);
        cells
            .binary_search_by_key(&class, |c| c.class)
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
    /// 门规则命中的可追溯规则身份。
    #[must_use]
    pub fn gate_attribution(
        self,
        gate: ManeuverGateOrdinal,
        class: ParticipantClassOrdinal,
    ) -> Option<PolicyRuleAttribution<'a>> {
        Some(self.attribution(self.gate(gate, class)?.rule))
    }
    /// 流规则命中的可追溯规则身份。
    #[must_use]
    pub fn stream_attribution(
        self,
        stream: ParticipantStreamOrdinal,
        class: ParticipantClassOrdinal,
    ) -> Option<PolicyRuleAttribution<'a>> {
        Some(self.attribution(self.stream(stream, class)?.rule))
    }
    /// 用 owner/class 查询，避免把另一共享根的解析 cell 混入此根。
    #[must_use]
    pub fn yield_targets(
        self,
        stream: ParticipantStreamOrdinal,
        class: ParticipantClassOrdinal,
        passage_local_index: u32,
    ) -> Option<(ConflictZoneOrdinal, &'a [YieldTargetCell])> {
        let cell = self.stream(stream, class)?;
        let range = cell
            .target_ranges
            .slice(&self.network.target_ranges)
            .get(passage_local_index as usize)?;
        Some((range.zone, range.targets.slice(&self.network.targets)))
    }
    /// 门规则关联的依据迭代器。
    pub fn gate_evidence(
        self,
        gate: ManeuverGateOrdinal,
        class: ParticipantClassOrdinal,
    ) -> Option<impl Iterator<Item = &'a PolicyEvidence>> {
        let rule = self.gate(gate, class)?.rule;
        Some(self.rule_evidence(rule))
    }
    /// 流规则关联的依据迭代器。
    pub fn stream_evidence(
        self,
        stream: ParticipantStreamOrdinal,
        class: ParticipantClassOrdinal,
    ) -> Option<impl Iterator<Item = &'a PolicyEvidence>> {
        let rule = self.stream(stream, class)?.rule;
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
