//! 借用预检索引的请求容量预算；来源 bytes 始终由调用方持有。

use std::cell::Cell;
use std::ops::{Deref, DerefMut};

use super::{CompileLimitDimension, CompileLimits, DiagnosticBundle, limit_error};

pub(super) struct PreflightScratch<'a> {
    limits: &'a CompileLimits,
    admitted_live_bytes: u64,
    live_bytes: Cell<u64>,
    peak_bytes: Cell<u64>,
}

impl<'a> PreflightScratch<'a> {
    pub(super) fn new(limits: &'a CompileLimits, admitted_live_bytes: u64) -> Self {
        Self {
            limits,
            admitted_live_bytes,
            live_bytes: Cell::new(0),
            peak_bytes: Cell::new(0),
        }
    }

    pub(super) fn peak_bytes(&self) -> u64 {
        self.peak_bytes.get()
    }

    pub(super) fn collect<T, I>(&self, values: I) -> Result<ScratchVec<'_, 'a, T>, DiagnosticBundle>
    where
        I: ExactSizeIterator<Item = T>,
    {
        let count = values.len();
        let stage_limit = self.limits.value(CompileLimitDimension::StageScratchBytes);
        let bytes = count
            .checked_mul(size_of::<T>())
            .filter(|bytes| *bytes <= isize::MAX as usize)
            .and_then(|bytes| u64::try_from(bytes).ok())
            .ok_or_else(|| {
                limit_error(
                    CompileLimitDimension::StageScratchBytes,
                    stage_limit,
                    u64::MAX,
                )
            })?;
        let live = self.live_bytes.get().checked_add(bytes).ok_or_else(|| {
            limit_error(
                CompileLimitDimension::StageScratchBytes,
                stage_limit,
                u64::MAX,
            )
        })?;
        if live > stage_limit {
            return Err(limit_error(
                CompileLimitDimension::StageScratchBytes,
                stage_limit,
                live,
            ));
        }
        let live_limit = self
            .limits
            .value(CompileLimitDimension::CompilerControlledLiveBytes);
        let total = self.admitted_live_bytes.checked_add(live).ok_or_else(|| {
            limit_error(
                CompileLimitDimension::CompilerControlledLiveBytes,
                live_limit,
                u64::MAX,
            )
        })?;
        if total > live_limit {
            return Err(limit_error(
                CompileLimitDimension::CompilerControlledLiveBytes,
                live_limit,
                total,
            ));
        }

        // 所有请求量先过预算；只请求一次精确容量，之后不增长。普通 Rust allocator
        // OOM 沿用前端既有行为，不伪装成来源语义错误或配置档超限。
        let mut entries = Vec::with_capacity(count);
        entries.extend(values);
        debug_assert_eq!(entries.len(), count, "exact-size private iterator");
        self.live_bytes.set(live);
        self.peak_bytes.set(self.peak_bytes.get().max(live));
        Ok(ScratchVec {
            entries,
            scratch: self,
            bytes,
        })
    }
}

impl Deref for PreflightScratch<'_> {
    type Target = CompileLimits;

    fn deref(&self) -> &Self::Target {
        self.limits
    }
}

pub(super) struct ScratchVec<'scratch, 'limits, T> {
    entries: Vec<T>,
    scratch: &'scratch PreflightScratch<'limits>,
    bytes: u64,
}

impl<T> Deref for ScratchVec<'_, '_, T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        &self.entries
    }
}

impl<T> DerefMut for ScratchVec<'_, '_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.entries
    }
}

impl<T> Drop for ScratchVec<'_, '_, T> {
    fn drop(&mut self) {
        self.scratch.live_bytes.set(
            self.scratch
                .live_bytes
                .get()
                .checked_sub(self.bytes)
                .expect("scratch release matches a live reservation"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DiagnosticPayload;

    #[test]
    fn budget_rejects_before_reading_or_allocating_entries() {
        let limits = CompileLimits::p100_initial_v1()
            .with_test_admission_limit(CompileLimitDimension::StageScratchBytes, 15);
        let scratch = PreflightScratch::new(&limits, 0);
        let reads = Cell::new(0);
        let error = match scratch.collect((0_u32..2).map(|_| {
            reads.set(reads.get() + 1);
            0_u64
        })) {
            Ok(_) => panic!("sixteen requested bytes exceed the scratch budget"),
            Err(error) => error,
        };
        assert_eq!(reads.get(), 0);
        assert_eq!(scratch.live_bytes.get(), 0);
        assert_eq!(scratch.peak_bytes(), 0);
        assert!(matches!(
            error.diagnostics()[0].payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::StageScratchBytes,
                limit: 15,
                observed: 16,
            }
        ));
    }

    #[test]
    fn simultaneous_requests_count_until_their_scopes_end() {
        let limits = CompileLimits::p100_initial_v1()
            .with_test_admission_limit(CompileLimitDimension::StageScratchBytes, 48);
        let scratch = PreflightScratch::new(&limits, 0);
        let first = scratch.collect((0_u32..4).map(u64::from)).unwrap();
        let second = scratch.collect((0_u32..2).map(u64::from)).unwrap();
        assert_eq!(scratch.live_bytes.get(), 48);
        assert!(scratch.collect((0_u32..1).map(u64::from)).is_err());
        assert_eq!(scratch.live_bytes.get(), 48);
        drop(second);
        let retry = scratch.collect((0_u32..2).map(u64::from)).unwrap();
        drop(first);
        drop(retry);
        assert_eq!(scratch.live_bytes.get(), 0);
        assert_eq!(scratch.peak_bytes(), 48);
    }

    #[test]
    fn existing_modules_consume_the_same_live_budget() {
        let limits = CompileLimits::p100_initial_v1()
            .with_test_admission_limit(CompileLimitDimension::CompilerControlledLiveBytes, 35);
        let scratch = PreflightScratch::new(&limits, 20);
        let error = match scratch.collect((0_u32..2).map(u64::from)) {
            Ok(_) => panic!("existing modules and scratch exceed the live budget"),
            Err(error) => error,
        };
        assert!(matches!(
            error.diagnostics()[0].payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::CompilerControlledLiveBytes,
                limit: 35,
                observed: 36,
            }
        ));
        assert_eq!(scratch.peak_bytes(), 0);
        let exact = PreflightScratch::new(&limits, 19);
        assert!(exact.collect((0_u32..2).map(u64::from)).is_ok());
    }

    #[test]
    fn unrepresentable_capacity_is_rejected_before_allocation() {
        let limits = CompileLimits::single_network_1m_v2();
        let scratch = PreflightScratch::new(&limits, 0);
        assert!(
            scratch
                .collect(std::iter::repeat_n(0_u64, usize::MAX))
                .is_err()
        );
        assert!(
            scratch
                .collect(std::iter::repeat_n(0_u8, isize::MAX as usize + 1))
                .is_err()
        );
        assert_eq!(scratch.peak_bytes(), 0);
    }
}
