// #801 导出树专用：每块独占一个记录；完整 join 后读取，逐车路径没有计时器。
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
pub(crate) struct ChunkTiming([AtomicU64; 6]);
impl ChunkTiming {
    pub(crate) fn store(&self, start: usize, len: usize, began: u64, ended: u64, worker: usize) {
        for (cell, value) in
            self.0
                .iter()
                .zip([start as u64, len as u64, began, ended, worker as u64, 1])
        {
            cell.store(value, Ordering::Relaxed);
        }
    }
    fn values(&self) -> [u64; 6] {
        std::array::from_fn(|i| self.0[i].load(Ordering::Relaxed))
    }
}
struct ChunkBatch {
    tick: u64,
    workload: usize,
    size: usize,
    dispatch_ns: u64,
    chunks: Vec<[u64; 6]>,
}
thread_local! {
    static CHUNK_REPORT: std::cell::RefCell<Option<ChunkBatch>> = const { std::cell::RefCell::new(None) };
}
pub(crate) fn save_chunks(
    tick: u64,
    workload: usize,
    size: usize,
    dispatch_ns: u64,
    records: &[ChunkTiming],
) {
    CHUNK_REPORT.with(|cell| {
        *cell.borrow_mut() = Some(ChunkBatch {
            tick,
            workload,
            size,
            dispatch_ns,
            chunks: records.iter().map(ChunkTiming::values).collect(),
        })
    });
}
fn emit_chunks() {
    if let Some(batch) = CHUNK_REPORT.with(|cell| cell.borrow_mut().take()) {
        eprintln!("LFP5 {{\"tick\":{},\"workload\":{},\"chunk_size\":{},\"dispatch_ns\":{},\"chunks\":{:?}}}", batch.tick, batch.workload, batch.size, batch.dispatch_ns, batch.chunks);
    }
}
