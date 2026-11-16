//! 测试构建的阶段归因支撑：占用四阶段批次墙钟（每批计时，无抽样）。

use stats_alloc::{INSTRUMENTED_SYSTEM, StatsAlloc};
use std::alloc::System;
use std::cell::RefCell;
use std::time::Instant;

// 仅单元测试二进制。配对墙钟同时报告分配计数；零分配稳态不触发分配器记账。
// 未插桩生产库墙钟仍使用独立的 runtime_profile_evidence 二进制。
#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

const STAGES: usize = 4;

#[derive(Clone, Copy)]
pub(crate) enum Stage {
    OccupancyCount,
    OccupancyLayout,
    OccupancyFill,
    OccupancySortSuffix,
}

#[derive(Clone, Default)]
struct Profile {
    enabled: bool,
    calls: [u64; STAGES],
    nanos: [u128; STAGES],
}

thread_local! {
    static PROFILE: RefCell<Profile> = RefCell::new(Profile::default());
}
crate::kernel::execution::carry_hooks!(carry_test_hooks: PROFILE);

pub(crate) struct Span(Option<(Stage, Instant)>);

pub(crate) fn begin(stage: Stage) -> Span {
    let timed = PROFILE.with_borrow_mut(|profile| {
        if !profile.enabled {
            return false;
        }
        profile.calls[stage as usize] += 1;
        true
    });
    Span(timed.then(|| (stage, Instant::now())))
}

impl Drop for Span {
    fn drop(&mut self) {
        if let Some((stage, started)) = self.0 {
            let elapsed = started.elapsed().as_nanos();
            PROFILE.with_borrow_mut(|profile| {
                profile.nanos[stage as usize] += elapsed;
            });
        }
    }
}

struct Session;

/// 测既有 Occupancy 四个批次的墙钟（每批计时）。
pub(crate) fn measure_batches<R>(run: impl FnOnce() -> R) -> (R, [u128; 4]) {
    let session = Session::start();
    let result = run();
    drop(session);
    (result, PROFILE.with_borrow(|profile| profile.nanos))
}

impl Session {
    fn start() -> Self {
        PROFILE.with_borrow_mut(|profile| {
            assert!(!profile.enabled, "research sessions must not nest");
            *profile = Profile {
                enabled: true,
                ..Profile::default()
            };
        });
        Self
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        PROFILE.with_borrow_mut(|profile| profile.enabled = false);
    }
}

#[test]
fn attribution_counts_calls_and_disables_on_unwind() {
    let unwind = std::panic::catch_unwind(|| {
        let _session = Session::start();
        for _ in 0..8 {
            drop(begin(Stage::OccupancyFill));
        }
        panic!("test session unwind");
    });
    assert!(unwind.is_err());
    PROFILE.with_borrow(|profile| {
        assert!(!profile.enabled);
        assert_eq!(profile.calls[Stage::OccupancyFill as usize], 8);
        assert!(profile.nanos[Stage::OccupancyFill as usize] > 0, "每批计时");
    });
    drop(begin(Stage::OccupancyFill));
    PROFILE.with_borrow(|profile| {
        assert_eq!(profile.calls[Stage::OccupancyFill as usize], 8);
    });
}
