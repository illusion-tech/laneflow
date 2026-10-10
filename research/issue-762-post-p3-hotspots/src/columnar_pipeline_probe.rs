// #814：仅追加到受检诊断导出树；逐行记录在线程内累加，任务结束才归约。
const PIPELINE_TIMES: usize = 29;
const PIPELINE_COUNTS: usize = 48;
struct PipelineLocal {
    work: [u64; 20],
    nanos: [u64; PIPELINE_TIMES],
    calls: [u64; PIPELINE_TIMES],
    counts: [u64; PIPELINE_COUNTS],
}
impl PipelineLocal {
    const fn new() -> Self {
        Self {
            work: [0; 20],
            nanos: [0; PIPELINE_TIMES],
            calls: [0; PIPELINE_TIMES],
            counts: [0; PIPELINE_COUNTS],
        }
    }
}
thread_local! {
    static PIPELINE_LOCAL: std::cell::RefCell<PipelineLocal> =
        const { std::cell::RefCell::new(PipelineLocal::new()) };
    static PIPELINE_SAMPLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
static COLUMNAR_WORK: [std::sync::atomic::AtomicU64; 20] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 20];
static PIPELINE_NANOS: [std::sync::atomic::AtomicU64; PIPELINE_TIMES] =
    [const { std::sync::atomic::AtomicU64::new(0) }; PIPELINE_TIMES];
static PIPELINE_CALLS: [std::sync::atomic::AtomicU64; PIPELINE_TIMES] =
    [const { std::sync::atomic::AtomicU64::new(0) }; PIPELINE_TIMES];
static PIPELINE_TOTALS: [std::sync::atomic::AtomicU64; PIPELINE_COUNTS] =
    [const { std::sync::atomic::AtomicU64::new(0) }; PIPELINE_COUNTS];
pub(crate) fn note_columnar_work(index: usize, amount: usize) {
    PIPELINE_LOCAL.with(|local| local.borrow_mut().work[index] += amount as u64);
}
pub(crate) fn note_pipeline(index: usize, amount: usize) {
    PIPELINE_LOCAL.with(|local| local.borrow_mut().counts[index] += amount as u64);
}
fn flush_pipeline() {
    use std::sync::atomic::Ordering;
    fn reduce<const N: usize>(local: [u64; N], totals: &[std::sync::atomic::AtomicU64; N]) {
        for (amount, total) in local.into_iter().zip(totals) {
            if amount != 0 {
                total.fetch_add(amount, Ordering::Relaxed);
            }
        }
    }
    let local = PIPELINE_LOCAL.with(|local| local.replace(PipelineLocal::new()));
    reduce(local.work, &COLUMNAR_WORK);
    reduce(local.nanos, &PIPELINE_NANOS);
    reduce(local.calls, &PIPELINE_CALLS);
    reduce(local.counts, &PIPELINE_TOTALS);
}
pub(crate) struct PipelineFlush;
impl Drop for PipelineFlush {
    fn drop(&mut self) {
        flush_pipeline();
    }
}
pub(crate) struct PipelineSample(bool);
impl PipelineSample {
    pub(crate) fn begin(enabled: bool) -> Self {
        Self(PIPELINE_SAMPLED.replace(enabled))
    }
}
impl Drop for PipelineSample {
    fn drop(&mut self) {
        PIPELINE_SAMPLED.set(self.0);
    }
}
pub(crate) struct PipelineTimer(Option<(usize, std::time::Instant)>);
impl PipelineTimer {
    pub(crate) fn begin(index: usize) -> Self {
        Self(Some((index, std::time::Instant::now())))
    }
    pub(crate) fn sampled(index: usize) -> Self {
        Self(
            PIPELINE_SAMPLED
                .with(|sampled| sampled.get().then(|| (index, std::time::Instant::now()))),
        )
    }
}
impl Drop for PipelineTimer {
    fn drop(&mut self) {
        if let Some((index, started)) = self.0 {
            let nanos = started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
            PIPELINE_LOCAL.with(|local| {
                let mut local = local.borrow_mut();
                local.nanos[index] += nanos;
                local.calls[index] += 1;
            });
        }
    }
}
pub(crate) fn take_columnar_work() -> [u64; 20] {
    flush_pipeline();
    std::array::from_fn(|index| COLUMNAR_WORK[index].swap(0, std::sync::atomic::Ordering::Relaxed))
}
pub(crate) fn take_pipeline() -> ([u64; 29], [u64; 29], [u64; 48]) {
    flush_pipeline();
    (
        std::array::from_fn(|index| {
            PIPELINE_NANOS[index].swap(0, std::sync::atomic::Ordering::Relaxed)
        }),
        std::array::from_fn(|index| {
            PIPELINE_CALLS[index].swap(0, std::sync::atomic::Ordering::Relaxed)
        }),
        std::array::from_fn(|index| {
            PIPELINE_TOTALS[index].swap(0, std::sync::atomic::Ordering::Relaxed)
        }),
    )
}
