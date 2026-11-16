//! 发射与检查路径接入资源账本（资源账本设计第 3.1、4 与 6 节）。
//!
//! 每次发射是一份发射账本，宿主直接调用的公开检查是一份检查账本；发射内部的检查以
//! 发射账本直接调用同一内部实现。各编译遍在开工前按已过格式预检的行数求出计划峰值，
//! 受控工作集只经编译遍的存储入口创建。

use crate::ledger::{BoundedVec, Ledger, LedgerError, LedgerStage, PassBudget, PassLabel, Plan};
use crate::{CompileLimitDimension, CompileLimits};

use super::PortableEmissionError;

impl From<LedgerError> for PortableEmissionError {
    fn from(value: LedgerError) -> Self {
        match value {
            LedgerError::BudgetExceeded {
                requested, limit, ..
            } => Self::CompileLimitExceeded {
                dimension: CompileLimitDimension::CompilerControlledLiveBytes,
                actual: requested,
                limit,
            },
            LedgerError::Defect { label, .. } => {
                Self::InternalAccountingDefect { point: label.point }
            }
            LedgerError::AllocationFailure { .. } => Self::AllocationFailure,
        }
    }
}

/// 本次顶层操作的账本，以及发射内部检查使用的阶段归属。
#[derive(Clone, Copy)]
pub(super) struct Accounting<'l> {
    pub(super) ledger: &'l Ledger,
    pub(super) stage: LedgerStage,
}

impl<'l> Accounting<'l> {
    /// 本阶段以 `point` 为记账点的编译遍标识。
    pub(super) const fn label(self, point: &'static str) -> PassLabel {
        PassLabel::new(self.stage, point)
    }

    /// 以 `point` 为记账点准入本阶段的顶层编译遍。
    pub(super) fn admit(
        self,
        point: &'static str,
        plan: Plan,
    ) -> Result<PassBudget<'l>, PortableEmissionError> {
        Ok(self
            .ledger
            .admit_pass(PassLabel::new(self.stage, point), plan)?)
    }
}

/// 宿主直接调用公开检查时新建检查账本，调用内部实现，返回前核对账本结束条件。
pub(super) fn with_check_ledger<T>(
    compile_limits: &CompileLimits,
    check: impl FnOnce(Accounting<'_>) -> Result<T, PortableEmissionError>,
) -> Result<T, PortableEmissionError> {
    let ledger = Ledger::new(compile_limits);
    let value = check(Accounting {
        ledger: &ledger,
        stage: LedgerStage::PublicCheck,
    })?;
    ledger.close()?;
    Ok(value)
}

/// 以编译遍授予创建可用逻辑容量为 `count` 的空序列。
pub(super) fn granted<T>(
    count: usize,
    pass: &PassBudget<'_>,
) -> Result<BoundedVec<T>, PortableEmissionError> {
    Ok(pass.vec(count)?)
}

/// 测试用会计上下文：账本泄漏为 `'static`，只在测试中使用。
#[cfg(test)]
pub(super) fn test_accounting(budget: u64) -> Accounting<'static> {
    Accounting {
        ledger: Box::leak(Box::new(Ledger::with_budget(budget))),
        stage: LedgerStage::SingleEmission,
    }
}

/// 测试用、额度不受限的编译遍。
#[cfg(test)]
pub(super) fn test_pass() -> PassBudget<'static> {
    test_accounting(u64::MAX)
        .admit("test", Plan::of(u64::MAX))
        .expect("不受限的测试编译遍")
}
