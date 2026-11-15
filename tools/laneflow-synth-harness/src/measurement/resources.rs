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
    let memory = laneflow_os_probe::Probe::new()?.memory()?;
    Ok(
        json!({"method":"laneflow-os-probe","pid":std::process::id(),
        "working_set_bytes":cfg!(windows).then_some(memory.resident_bytes).flatten(),
        "peak_working_set_bytes":cfg!(windows).then_some(memory.peak_resident_bytes).flatten(),
        "private_bytes":memory.windows_private_bytes,
        "commit_peak_bytes":memory.windows_peak_commit_bytes,
        "page_fault_count":memory.page_faults,
        "linux_vm_rss_bytes":cfg!(target_os = "linux").then_some(memory.resident_bytes).flatten(),
        "linux_vm_hwm_bytes":cfg!(target_os = "linux").then_some(memory.peak_resident_bytes).flatten(),
        "linux_rss_anon_bytes":memory.linux_anonymous_resident_bytes,
        "linux_major_page_fault_count":memory.linux_major_page_faults,
        "peak_scope":"process_lifetime_includes_initialization_and_evidence_buffers"}),
    )
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
