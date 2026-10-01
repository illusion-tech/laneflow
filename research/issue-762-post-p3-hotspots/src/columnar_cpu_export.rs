//! 仅诊断导出树记录块内耗时；并行块之和不等于阶段墙钟或线程 CPU 时间。
use crate::{Result, columnar_export::patch};
use std::{fs, path::Path};

const MOTION: &str = "crates/laneflow-runtime/src/kernel/tick/columnar_motion.rs";

pub(crate) fn instrument(root: &Path, candidate: bool) -> Result<()> {
    if candidate {
        for (old, new) in [
            (
                "    let mut batch = Batch::new();\n    let n = chunk.cursor.len();",
                "    let _prepare_timer = BlockTimer::begin(0);\n    let mut batch = Batch::new();\n    let n = chunk.cursor.len();",
            ),
            (
                "    if !batch.enabled[..n].iter().any(|&enabled| enabled) {",
                "    drop(_prepare_timer);\n    if !batch.enabled[..n].iter().any(|&enabled| enabled) {",
            ),
            (
                "    let n = chunk.cursor.len();\n    let (proposal_speed, proposal_travel, out_speed, out_travel)",
                "    let _numeric_timer = BlockTimer::begin(1);\n    let n = chunk.cursor.len();\n    let (proposal_speed, proposal_travel, out_speed, out_travel)",
            ),
            (
                "    let n = chunk.cursor.len();\n    for row in 0..n {\n        batch.drop_index[row]",
                "    let _limit_timer = BlockTimer::begin(2);\n    let n = chunk.cursor.len();\n    for row in 0..n {\n        batch.drop_index[row]",
            ),
            (
                "    // 工作掩码驱动真实多跳行走；",
                "    let _walk_timer = BlockTimer::begin(3);\n    // 工作掩码驱动真实多跳行走；",
            ),
            (
                "    for (row, route) in routes.iter().enumerate().take(n) {\n        if !batch.enabled[row] || chunk.reports[row].error.is_some() {",
                "    drop(_walk_timer);\n    let _finalize_timer = BlockTimer::begin(4);\n    for (row, route) in routes.iter().enumerate().take(n) {\n        if !batch.enabled[row] || chunk.reports[row].error.is_some() {",
            ),
        ] {
            patch(root, MOTION, old, new)?;
        }
        let path = root.join(MOTION);
        let mut text = fs::read_to_string(&path)?;
        text.push_str(
            r#"
static BLOCK_ELAPSED_NS: [std::sync::atomic::AtomicU64; 5] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 5];
struct BlockTimer { index: usize, started: std::time::Instant }
impl BlockTimer {
    fn begin(index: usize) -> Self { Self { index, started: std::time::Instant::now() } }
}
impl Drop for BlockTimer {
    fn drop(&mut self) {
        let elapsed = self.started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
        BLOCK_ELAPSED_NS[self.index].fetch_add(elapsed, std::sync::atomic::Ordering::Relaxed);
    }
}
pub(super) fn take_block_elapsed_ns() -> [u64; 5] {
    std::array::from_fn(|i| BLOCK_ELAPSED_NS[i].swap(0, std::sync::atomic::Ordering::Relaxed))
}
"#,
        );
        fs::write(path, text)?;
    }
    let tick = root.join("crates/laneflow-runtime/src/kernel/tick.rs");
    let mut text = fs::read_to_string(&tick)?;
    text.push_str(if candidate {
        "\npub(crate) fn take_block_elapsed_ns() -> [u64; 5] { columnar_motion::take_block_elapsed_ns() }\n"
    } else {
        "\npub(crate) fn take_block_elapsed_ns() -> [u64; 5] { [0; 5] }\n"
    });
    fs::write(tick, text)?;
    let lib = root.join("crates/laneflow-runtime/src/lib.rs");
    let mut text = fs::read_to_string(&lib)?;
    text.push_str("\n#[doc(hidden)] pub fn research_block_elapsed_ns() -> [u64; 5] { kernel::tick::take_block_elapsed_ns() }\n");
    fs::write(lib, text)?;
    patch(
        root,
        "tools/laneflow-urban-harness/src/host.rs",
        "                laneflow_runtime::research_work();",
        "                laneflow_runtime::research_work();\n                laneflow_runtime::research_block_elapsed_ns();",
    )?;
    patch(
        root,
        "tools/laneflow-urban-harness/src/host.rs",
        "                let (stages, calls) = laneflow_runtime::research_take();",
        "                let (stages, calls) = laneflow_runtime::research_take();\n                eprintln!(\"LF814_BLOCK {{\\\"tick\\\":{},\\\"elapsed_sum_ns\\\":{:?}}}\", world.tick_index(), laneflow_runtime::research_block_elapsed_ns());",
    )?;
    Ok(())
}
