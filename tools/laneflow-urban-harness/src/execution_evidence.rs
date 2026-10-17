//! #972 证据构建：观察窗口的调度统计与资源账，写入 `diagnostics.json`。
//! 计时含插桩开销，不作为性能认证。

use laneflow_runtime::{ExecutionEvidence, ResourceEvidence};
use serde_json::{Value, json};

use crate::Result;
use crate::host::Host;

/// 从窗口起点开始累计；预热结束时清零重开。
pub(crate) struct Window {
    from_tick: u64,
    resources_at_start: ResourceEvidence,
}

impl Window {
    /// 丢弃此前累计，从 `tick` 之后开始计。
    pub(crate) fn start(world: &mut Host, tick: u64) -> Result<Self> {
        world.take_execution_evidence()?;
        Ok(Self {
            from_tick: tick,
            resources_at_start: world.resource_evidence(),
        })
    }

    /// `completed_tick` 是最后一个完整记账的拍；步进或其后记账失败时，账本已含
    /// 那次尝试，尝试边界按步进次数另报。
    pub(crate) fn finish(self, world: &mut Host, completed_tick: u64) -> Result<Value> {
        let evidence = world.take_execution_evidence()?;
        Ok(json!({
            "purpose": "instrumented-evidence-not-performance-certification",
            "after_tick": self.from_tick,
            "completed_through_tick": completed_tick,
            "attempted_through_tick": self.from_tick.saturating_add(evidence.steps),
            "schedule": schedule(&evidence),
            "resources": {
                "window_start": resources(&self.resources_at_start),
                "window_end": resources(&world.resource_evidence()),
            },
        }))
    }
}

fn schedule(evidence: &ExecutionEvidence) -> Value {
    json!({
        "steps": evidence.steps,
        "step_wall_ns": evidence.step_wall_ns,
        "stages": evidence.stages.iter().map(|stage| {
            let dispatch = &stage.dispatch;
            json!({
                "stage": stage.stage,
                "calls": stage.calls,
                "wall_ns": stage.wall_ns,
                "dispatches": dispatch.dispatches,
                "inline_dispatches": dispatch.inline_dispatches,
                "dispatch_wall_ns": dispatch.wall_ns,
                "inline_wall_ns": dispatch.inline_wall_ns,
                "blocks": dispatch.blocks,
                "skipped_blocks": dispatch.skipped_blocks,
                "extra_work_blocks": dispatch.extra_work_blocks,
                "thread_slots": dispatch.thread_slots,
                "busy_ns": dispatch.busy_ns,
                "critical_path_ns": dispatch.critical_path_ns,
                "max_block_ns": dispatch.max_block_ns,
                "output_bytes": dispatch.output_bytes,
            })
        }).collect::<Vec<_>>(),
        "routes": evidence.routes.iter().map(|route| json!({
            "phase": route.phase,
            "dispatched": route.dispatched,
            "fused_serial_executor": route.fused_serial_executor,
            "fused_below_threshold": route.fused_below_threshold,
            "fused_empty": route.fused_empty,
            "scratch_fallback": route.scratch_fallback,
        })).collect::<Vec<_>>(),
    })
}

fn resources(resources: &ResourceEvidence) -> Value {
    json!({
        "workers": resources.workers,
        "pool_threads": resources.pool_threads,
        "worker_stack_requested_bytes": resources.worker_stack_requested_bytes,
        "worker_stack_reserved_bytes": null,
        "worker_stack_committed_bytes": null,
        "shared_network_bytes": resources.shared_network_bytes,
        "binding_bytes": resources.binding_bytes,
        "committed_bytes": resources.committed_bytes,
        "derived_bytes": resources.derived_bytes,
        "workspace_bytes": resources.workspace_bytes,
        "administrative_bytes": resources.administrative_bytes,
        "active_plan_bytes": resources.active_plan_bytes,
        "coordinator_scratch_bytes": resources.coordinator_scratch_bytes,
    })
}
