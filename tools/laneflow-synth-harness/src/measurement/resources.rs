use laneflow_runtime::TrafficWorld;
use serde::Serialize;
use serde_json::{Value, json};

use crate::Error;

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub(super) struct Allocation {
    pub allocations: usize,
    pub deallocations: usize,
    pub reallocations: usize,
    pub bytes_allocated: usize,
    pub bytes_deallocated: usize,
    pub bytes_reallocated: isize,
}

#[cfg(feature = "allocation")]
pub(super) type AllocationStart = stats_alloc::Stats;
#[cfg(not(feature = "allocation"))]
#[derive(Clone, Copy)]
pub(super) struct AllocationStart;

pub(super) fn start() -> AllocationStart {
    #[cfg(feature = "allocation")]
    {
        stats_alloc::INSTRUMENTED_SYSTEM.stats()
    }
    #[cfg(not(feature = "allocation"))]
    {
        AllocationStart
    }
}

pub(super) fn finish(before: AllocationStart) -> Option<Allocation> {
    #[cfg(feature = "allocation")]
    {
        let delta = stats_alloc::INSTRUMENTED_SYSTEM.stats() - before;
        Some(Allocation {
            allocations: delta.allocations,
            deallocations: delta.deallocations,
            reallocations: delta.reallocations,
            bytes_allocated: delta.bytes_allocated,
            bytes_deallocated: delta.bytes_deallocated,
            bytes_reallocated: delta.bytes_reallocated,
        })
    }
    #[cfg(not(feature = "allocation"))]
    {
        let _ = before;
        None
    }
}

pub(super) fn process() -> Result<Value, Error> {
    #[cfg(windows)]
    {
        let p = win32job::utils::get_process_memory_info(win32job::utils::get_current_process())?;
        Ok(
            json!({"method":"GetProcessMemoryInfo/PROCESS_MEMORY_COUNTERS_EX",
            "pid":std::process::id(),"working_set_bytes":p.working_set_size,
            "peak_working_set_bytes":p.peak_working_set_size,"private_bytes":p.private_usage,
            "commit_peak_bytes":p.peak_pagefile_usage,"page_fault_count":p.page_fault_count,
            "peak_scope":"process_lifetime_includes_initialization_and_evidence_buffers"}),
        )
    }
    #[cfg(not(windows))]
    {
        Ok(json!({"method":"unavailable_on_this_platform","pid":std::process::id()}))
    }
}

pub(super) fn runtime(world: &TrafficWorld) -> Value {
    #[cfg(feature = "allocation")]
    {
        let r = world.resource_evidence();
        json!({"method":"execution_evidence_owned_capacity_ledger_not_heap_peak",
            "workers":r.workers,"pool_threads":r.pool_threads,
            "worker_stack_requested_bytes":r.worker_stack_requested_bytes,
            "shared_network_bytes":r.shared_network_bytes,"binding_bytes":r.binding_bytes,
            "committed_bytes":r.committed_bytes,"derived_bytes":r.derived_bytes,
            "workspace_bytes":r.workspace_bytes,"administrative_bytes":r.administrative_bytes,
            "active_plan_bytes":r.active_plan_bytes,"coordinator_scratch_bytes":r.coordinator_scratch_bytes})
    }
    #[cfg(not(feature = "allocation"))]
    {
        let _ = world;
        Value::Null
    }
}
