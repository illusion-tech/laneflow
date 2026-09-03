//! 临时行数不能约束未命中的查询；关系遍历另按同一上限累计实际工作量。
use super::*;
use crate::CompileLimits;

pub(super) struct WorkBudget {
    used: u64,
    limit: u64,
}

impl WorkBudget {
    pub(super) fn new(limits: &CompileLimits) -> Self {
        Self {
            used: 0,
            limit: limits.value(CompileLimitDimension::RelationOccurrenceCount),
        }
    }

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

    #[cfg(test)]
    pub(super) fn used(&self) -> u64 {
        self.used
    }
}
