//! #762 导出研究树专用的协调器阶段墙钟；正式 Runtime 不编译此文件。
use std::cell::Cell;
use std::time::Instant;

#[derive(Clone, Copy)]
pub(crate) enum Stage {
    Preflight,
    Occupancy,
    WaitingPrepare,
    ConflictPrepare,
    MotionLoop,
    WaitingFinalize,
    Signals,
    ConflictFinalize,
    WaitingOutputs,
    Commit,
    Frontier,
    P4,
}

thread_local! {
    static NANOS: Cell<[u128; 12]> = const { Cell::new([0; 12]) };
    static CALLS: Cell<[u64; 12]> = const { Cell::new([0; 12]) };
}

pub(crate) struct Span(Stage, Instant);

pub(crate) fn begin(stage: Stage) -> Span {
    Span(stage, Instant::now())
}

impl Drop for Span {
    fn drop(&mut self) {
        let elapsed = self.1.elapsed().as_nanos();
        NANOS.with(|cell| {
            let mut values = cell.get();
            values[self.0 as usize] += elapsed;
            cell.set(values);
        });
        CALLS.with(|cell| {
            let mut values = cell.get();
            values[self.0 as usize] += 1;
            cell.set(values);
        });
    }
}

pub(crate) fn reset() {
    NANOS.set([0; 12]);
    CALLS.set([0; 12]);
}

pub(crate) fn take() -> ([u128; 12], [u64; 12]) {
    (NANOS.get(), CALLS.get())
}
