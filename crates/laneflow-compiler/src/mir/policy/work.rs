//! 临时行数不能约束未命中的查询；关系遍历另按同一上限累计实际工作量。
use super::*;
use crate::CompileLimits;

/// 策略校验的关系遍历工作预算，按统一上限累计实际工作量。
pub(super) struct WorkBudget {
    used: u64,
    limit: u64,
}

impl WorkBudget {
    /// 以 CompileLimits 的关系出现数上限新建工作预算。
    pub(super) fn new(limits: &CompileLimits) -> Self {
        Self {
            used: 0,
            limit: limits.value(CompileLimitDimension::RelationOccurrenceCount),
        }
    }

    /// 计入 `count` 单位工作量；累计超过上限时返回超限诊断。
    pub(super) fn charge(&mut self, count: u64) -> Result<(), DiagnosticBundle> {
        let observed = self.used.saturating_add(count);
        if observed > self.limit {
            return Err(DiagnosticBundle::single(
                Diagnostic::compile_limit_exceeded(
                    CompileLimitDimension::RelationOccurrenceCount,
                    self.limit,
                    observed,
                ),
            ));
        }
        self.used = observed;
        Ok(())
    }

    /// 返回已累计的工作量（仅测试用）。
    #[cfg(test)]
    pub(super) fn used(&self) -> u64 {
        self.used
    }
}
