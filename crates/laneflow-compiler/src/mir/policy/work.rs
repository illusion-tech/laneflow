//! 策略访问预算独立于来源关系与临时存储；未命中查询仍消费工作量。
use super::*;
use crate::CompileLimits;

/// 一次策略校验的累计访问预算；工作量不随临时存储释放归还。
pub(super) struct WorkBudget {
    used: u64,
    limit: u64,
}

impl WorkBudget {
    /// 以配置档的独立操作工作量预算开始一次校验。
    pub(super) fn new(limits: &CompileLimits) -> Self {
        Self {
            used: 0,
            limit: limits.value(CompileLimitDimension::PolicyValidationWorkUnits),
        }
    }

    /// 计入 `count` 单位工作量；累计超过上限时返回超限诊断。
    pub(super) fn charge(&mut self, count: u64) -> Result<(), DiagnosticBundle> {
        let observed = self.used.saturating_add(count);
        if observed > self.limit {
            return Err(DiagnosticBundle::single(
                Diagnostic::compile_limit_exceeded(
                    CompileLimitDimension::PolicyValidationWorkUnits,
                    self.limit,
                    observed,
                ),
            ));
        }
        self.used = observed;
        Ok(())
    }

    /// 返回本次校验已消费的工作量。
    pub(super) fn used(&self) -> u64 {
        self.used
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn charges_accumulate_and_reject_before_crossing_the_budget() {
        let limits = CompileLimits::p100().with_test_policy_work_limit(5);
        let mut work = WorkBudget::new(&limits);
        work.charge(3).unwrap();
        work.charge(2).unwrap();
        assert_eq!(work.used(), 5);
        let error = work.charge(1).unwrap_err();
        assert!(matches!(
            error.diagnostics()[0].payload(),
            crate::DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::PolicyValidationWorkUnits,
                limit: 5,
                observed: 6,
            }
        ));
        assert_eq!(work.used(), 5);
        assert!(work.charge(u64::MAX).is_err());
        let mut next_operation = WorkBudget::new(&limits);
        next_operation.charge(5).unwrap();
    }
}
