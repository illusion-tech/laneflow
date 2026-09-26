//! 独立分配构建；进程级 allocator 计数不用于正常墙钟结论。

#[global_allocator]
pub(crate) static ALLOCATOR: &stats_alloc::StatsAlloc<std::alloc::System> =
    &stats_alloc::INSTRUMENTED_SYSTEM;
