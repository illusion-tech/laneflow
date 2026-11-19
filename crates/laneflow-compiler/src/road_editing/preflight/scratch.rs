//! 借用预检索引的存储入口：索引经预检编译遍的授予创建；来源 bytes 始终由调用方持有。

use std::ops::Deref;

use crate::ledger::{BoundedVec, PassBudget};

use super::{CompileProfile, DiagnosticBundle};

/// 预检编译遍的索引入口：计划已在预检开始前按 wire 计数准入，索引只在计划内取得授予。
pub(super) struct PreflightScratch<'a> {
    limits: &'a CompileProfile,
    pass: &'a PassBudget<'a>,
}

impl<'a> PreflightScratch<'a> {
    pub(super) const fn new(limits: &'a CompileProfile, pass: &'a PassBudget<'a>) -> Self {
        Self { limits, pass }
    }

    /// 以授予创建恰好容纳 `values` 全部元素的索引；超出预检计划属记账缺陷。
    pub(super) fn collect<T, I>(&self, values: I) -> Result<BoundedVec<T>, DiagnosticBundle>
    where
        I: ExactSizeIterator<Item = T>,
    {
        Ok(self.pass.collect(values)?)
    }
}

impl Deref for PreflightScratch<'_> {
    type Target = CompileProfile;

    fn deref(&self) -> &Self::Target {
        self.limits
    }
}

/// 测试用：额度不受限的预检入口。
#[cfg(test)]
pub(super) fn unlimited(limits: &CompileProfile) -> PreflightScratch<'_> {
    let pass: &'static PassBudget<'static> = Box::leak(Box::new(crate::ledger::test_pass()));
    PreflightScratch::new(limits, pass)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes_hold_grants_until_their_scopes_end() {
        let limits = CompileProfile::single_network_10k_stable_entities();
        let ledger = crate::ledger::Ledger::with_budget(48);
        let pass = ledger
            .admit_pass(
                crate::ledger::PassLabel::new(
                    crate::ledger::LedgerStage::Admission,
                    "preflight-test",
                ),
                crate::ledger::Plan::ZERO.array::<u64>(6),
            )
            .unwrap();
        let scratch = PreflightScratch::new(&limits, &pass);
        let first = scratch.collect((0_u32..4).map(u64::from)).unwrap();
        let second = scratch.collect((0_u32..2).map(u64::from)).unwrap();
        assert_eq!(ledger.live(), 48);
        // 计划之外的索引属记账缺陷，不是预算诊断。
        let Err(defect) = scratch.collect((0_u32..1).map(u64::from)) else {
            panic!("an index outside the plan is an accounting defect");
        };
        assert!(matches!(
            defect.diagnostics()[0].payload(),
            crate::DiagnosticPayload::ResourceAccountingDefect { .. }
        ));
        drop(second);
        let retry = scratch.collect((0_u32..2).map(u64::from)).unwrap();
        drop((first, retry));
        assert_eq!(ledger.live(), 0);
    }
}
