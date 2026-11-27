//! 准入规则的类别成员：原始引用决定暂存上界，逐规则去重后的行决定输出区间。

use crate::SourceLocation;
use crate::arena::{ArenaPushError, TableRange};
use crate::ledger::{BoundedVec, Frozen, LedgerError, PassBudget, Plan};

use super::{AccessCandidate, HirAccessRuleParticipantClass, HirParticipantClassKey};

pub(super) type ResolvedRuleClass = (HirParticipantClassKey, SourceLocation);

#[derive(Default)]
pub(in crate::hir) struct RuleClassMemberPlan {
    raw_references: usize,
    largest_rule: usize,
}

impl RuleClassMemberPlan {
    pub(in crate::hir) fn record_rule(&mut self, references: usize) {
        self.raw_references = self.raw_references.saturating_add(references);
        self.largest_rule = self.largest_rule.max(references);
    }

    pub(in crate::hir) fn output(&self) -> Plan {
        Plan::ZERO.array::<HirAccessRuleParticipantClass>(self.raw_references)
    }

    /// 持久预算已留出一张成员表：解析时容纳累计行，复制及查歧义时容纳精确输出。
    /// 各步另需的单规则解析键、复制时的累计行和查歧义候选互不同时存续。
    pub(in crate::hir) fn scratch(&self) -> Plan {
        Plan::ZERO
            .array::<ResolvedRuleClass>(self.largest_rule)
            .max(self.output())
            .max(Plan::ZERO.array::<AccessCandidate>(self.raw_references))
    }

    pub(super) fn stage(&self, pass: &PassBudget<'_>) -> Result<StagedRuleMembers, LedgerError> {
        Ok(StagedRuleMembers {
            rows: pass.vec(self.raw_references)?,
        })
    }
}

pub(super) struct StagedRuleMembers {
    rows: BoundedVec<HirAccessRuleParticipantClass>,
}

impl StagedRuleMembers {
    pub(super) fn append_rule(
        &mut self,
        mut resolved: BoundedVec<ResolvedRuleClass>,
    ) -> Result<TableRange<HirAccessRuleParticipantClass>, ArenaPushError> {
        resolved.sort_unstable_by_key(|(class, _)| *class);
        resolved.dedup_by_key(|(class, _)| *class);
        let range = TableRange::try_from_usize(self.rows.len(), resolved.len())?;
        for (participant_class, source_span) in resolved.drain() {
            self.rows.push(HirAccessRuleParticipantClass {
                participant_class,
                source_span,
            })?;
        }
        Ok(range)
    }

    pub(super) fn freeze(
        mut self,
        pass: &PassBudget<'_>,
    ) -> Result<Frozen<HirAccessRuleParticipantClass>, LedgerError> {
        pass.collect(self.rows.drain())?.freeze()
    }
}

#[cfg(test)]
mod tests;
