//! 与墙钟采样分开的分配记账进程；相同输入和生产调用链。
#[global_allocator]
static ALLOCATOR: &stats_alloc::StatsAlloc<std::alloc::System> = &stats_alloc::INSTRUMENTED_SYSTEM;

#[path = "support/junction_scale.rs"]
mod scale;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    scale::run(true, false)
}
