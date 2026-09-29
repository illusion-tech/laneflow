// #772 仅拼接到隔离诊断树；工作线程计时和全量计数按块归约。
static COUNTERS: [std::sync::atomic::AtomicU64; 17] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 17];
static WORK_NANOS: [std::sync::atomic::AtomicU64; 15] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 15];
static WORK_CALLS: [std::sync::atomic::AtomicU64; 15] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 15];
thread_local! {
    static COUNTS: Cell<[u64;17]> = const { Cell::new([0;17]) };
    static SAMPLED: Cell<bool> = const {Cell::new(false)};
    static IN_P5: Cell<bool> = const {Cell::new(false)};
}
pub(crate) struct SampleScope(bool, bool);
pub(crate) fn sample_scope(index: usize, tick: u64) -> SampleScope {
    let selected = (index as u64) % 64 == tick % 64;
    let guard = SampleScope(IN_P5.replace(true), SAMPLED.replace(selected));
    count_by(0, 1);
    if selected {
        count_by(3, 1);
    }
    guard
}
impl Drop for SampleScope {
    fn drop(&mut self) {
        IN_P5.set(self.0);
        SAMPLED.set(self.1);
    }
}
pub(crate) fn sample(stage: Stage) -> Option<Span> {
    SAMPLED.get().then(|| begin(stage))
}
pub(crate) fn count_by(index: usize, value: u64) {
    COUNTS.with(|cell| {
        let mut v = cell.get();
        v[index] += value;
        cell.set(v);
    });
}
pub(crate) fn count_motion(index: usize) {
    if IN_P5.get() {
        count_by(index, 1);
    }
}
pub(crate) fn classify(reused: bool) {
    count_by(if reused { 1 } else { 2 }, 1);
    if SAMPLED.get() {
        count_by(if reused { 4 } else { 5 }, 1);
    }
}
pub(crate) fn flush_p5() {
    use std::sync::atomic::Ordering::Relaxed;
    for (counter, value) in COUNTERS.iter().zip(COUNTS.replace([0; 17])) {
        if value != 0 {
            counter.fetch_add(value, Relaxed);
        }
    }
    let mut times = NANOS.get();
    let mut calls = CALLS.get();
    for i in 0..15 {
        if times[23 + i] != 0 {
            WORK_NANOS[i].fetch_add(
                u64::try_from(times[23 + i]).expect("sample clock fits u64"),
                Relaxed,
            );
        }
        if calls[23 + i] != 0 {
            WORK_CALLS[i].fetch_add(calls[23 + i], Relaxed);
        }
        times[23 + i] = 0;
        calls[23 + i] = 0;
    }
    NANOS.set(times);
    CALLS.set(calls);
}
pub(crate) fn reset_p5() {
    COUNTS.set([0; 17]);
    SAMPLED.set(false);
    IN_P5.set(false);
    for a in [&COUNTERS[..], &WORK_NANOS[..], &WORK_CALLS[..]] {
        for c in a {
            c.store(0, std::sync::atomic::Ordering::Relaxed);
        }
    }
}
pub(crate) fn take_p5() -> ([u128; 55], [u64; 55]) {
    flush_p5();
    let mut times = NANOS.get();
    let mut calls = CALLS.get();
    for i in 0..15 {
        times[23 + i] = u128::from(WORK_NANOS[i].load(std::sync::atomic::Ordering::Relaxed));
        calls[23 + i] = WORK_CALLS[i].load(std::sync::atomic::Ordering::Relaxed);
    }
    for (i, c) in COUNTERS.iter().enumerate() {
        calls[38 + i] = c.load(std::sync::atomic::Ordering::Relaxed);
    }
    (times, calls)
}
