use super::*;

#[derive(Clone, Copy, Default, Serialize)]
struct Allocation {
    regions: u64,
    allocations: usize,
    deallocations: usize,
    reallocations: usize,
    bytes_allocated: usize,
    bytes_deallocated: usize,
    bytes_reallocated: isize,
}

impl Allocation {
    fn add(&mut self, delta: stats_alloc::Stats) {
        self.regions += 1;
        self.allocations += delta.allocations;
        self.deallocations += delta.deallocations;
        self.reallocations += delta.reallocations;
        self.bytes_allocated += delta.bytes_allocated;
        self.bytes_deallocated += delta.bytes_deallocated;
        self.bytes_reallocated += delta.bytes_reallocated;
    }
}

#[derive(Clone, Copy, Default, Serialize)]
struct ScanCount {
    invocations: u64,
    visited_rows: u64,
}

#[derive(Clone, Copy, Default)]
struct Work {
    boundaries: u64,
    scans: [ScanCount; 10],
    body_fragments: u64,
    maximum_body_fragments_per_boundary: u64,
}

#[derive(Clone, Copy, Default, Serialize)]
struct HighWater {
    observed: bool,
    initial_capacity: usize,
    maximum_length: usize,
    maximum_capacity: usize,
    element_bytes: usize,
    capacity_high_water_increases: u64,
    capacity_high_water_increase_bytes: usize,
}

pub(super) struct Counts {
    common: Common,
    allocation: [Allocation; 7],
    work: [Work; 3],
    buffers: [HighWater; 6],
    scope: usize,
    body_fragments: u64,
}

impl Counts {
    pub(super) fn new(implementation: &str, execution: ExecutionConfig) -> Self {
        Self {
            common: Common::new(Mode::Counts, implementation, execution),
            allocation: [Allocation::default(); 7],
            work: [Work::default(); 3],
            buffers: [HighWater::default(); 6],
            scope: 0,
            body_fragments: 0,
        }
    }
}

impl Probe for Counts {
    type Mark = stats_alloc::Stats;

    fn start(&mut self) -> Self::Mark {
        stats_alloc::INSTRUMENTED_SYSTEM.stats()
    }

    fn end(&mut self, phase: Phase, mark: Self::Mark) {
        // 读取差值先于任何记录转换；计量器只更新预先拥有的标量数组。
        let delta = stats_alloc::INSTRUMENTED_SYSTEM.stats() - mark;
        self.allocation[phase as usize].add(delta);
    }

    fn output_created(&mut self, output: &Path) -> Result<(), Error> {
        self.common.output_created(output)
    }

    fn bind(&mut self, bundle: &Bundle, plan: &Plan) -> Result<(), Error> {
        self.common.bind(bundle, plan)
    }

    fn boundary(&mut self, tick: u64) {
        self.scope = self.common.scope(tick);
        self.body_fragments = 0;
    }

    fn boundary_complete(&mut self) {
        self.common.completed_boundaries += 1;
        let work = &mut self.work[self.scope];
        work.boundaries += 1;
        work.maximum_body_fragments_per_boundary = work
            .maximum_body_fragments_per_boundary
            .max(self.body_fragments);
    }

    fn scan(&mut self, scan: Scan) {
        self.work[self.scope].scans[scan as usize].invocations += 1;
    }

    fn visit(&mut self, scan: Scan, rows: usize) {
        self.work[self.scope].scans[scan as usize].visited_rows += rows as u64;
    }

    fn body_fragment(&mut self) {
        self.work[self.scope].body_fragments += 1;
        self.body_fragments += 1;
    }

    fn buffer(&mut self, buffer: Buffer, length: usize, capacity: usize, item_bytes: usize) {
        let high = &mut self.buffers[buffer as usize];
        if !high.observed {
            high.observed = true;
            high.initial_capacity = capacity;
            high.element_bytes = item_bytes;
        } else if capacity > high.maximum_capacity {
            high.capacity_high_water_increases += 1;
            high.capacity_high_water_increase_bytes +=
                (capacity - high.maximum_capacity) * item_bytes;
        }
        high.maximum_length = high.maximum_length.max(length);
        high.maximum_capacity = high.maximum_capacity.max(capacity);
    }
}

impl ReportProbe for Counts {
    fn common(&self) -> &Common {
        &self.common
    }

    fn stop(&mut self) {}

    fn report(&self, complete: bool) -> Result<Value, Error> {
        let mut report = self.common.report(complete);
        report["allocation"] = json!({
            "method":"stats_alloc_process_global_nonoverlapping_regions",
            "coverage":"named_phase_regions_only_not_whole_run_allocation",
            "excluded":"input_verification_plan_read_meter_binding_final_event_and_report_writes_cost_serialization",
            "concurrent_unrelated_process_threads_invalidate_exact_attribution":true,
            "cost_metadata_sample_storage_and_serialization_excluded":true,
            "phases":Phase::ALL.into_iter().map(|phase| {
                (phase.name().to_owned(),json!(self.allocation[phase as usize]))
            }).collect::<serde_json::Map<_,_>>()
        });
        report["workload_counts"] = SCOPES
            .into_iter()
            .enumerate()
            .map(|(index, scope)| {
                let work = self.work[index];
                (
                    scope.to_owned(),
                    json!({
                        "completed_boundaries":work.boundaries,
                        "body_fragments":work.body_fragments,
                        "maximum_body_fragments_per_boundary":work.maximum_body_fragments_per_boundary,
                        "scans":Scan::ALL.into_iter().map(|scan| {
                            (scan.name().to_owned(),json!(work.scans[scan as usize]))
                        }).collect::<serde_json::Map<_,_>>()
                    }),
                )
            })
            .collect::<serde_json::Map<_, _>>()
            .into();
        report["scratch_high_water"] = Buffer::ALL
            .into_iter()
            .filter(|buffer| self.buffers[*buffer as usize].observed)
            .map(|buffer| {
                let high = self.buffers[buffer as usize];
                (
                    buffer.name().to_owned(),
                    json!({"initial_capacity":high.initial_capacity,"maximum_length":high.maximum_length,
                        "maximum_capacity":high.maximum_capacity,"element_bytes":high.element_bytes,
                        "retained_capacity_bytes":high.maximum_capacity*high.element_bytes,
                        "capacity_high_water_increases":high.capacity_high_water_increases,
                        "capacity_high_water_increase_bytes":high.capacity_high_water_increase_bytes}),
                )
            })
            .collect::<serde_json::Map<_, _>>()
            .into();
        Ok(report)
    }
}

pub(super) fn require_allocator() -> Result<(), Error> {
    // 库调用方未把 INSTRUMENTED_SYSTEM 装为 global_allocator 时，读取它
    // 会得到看似合法的零。实分配探针只用于准入，在所有受计区间之外。
    let before = stats_alloc::INSTRUMENTED_SYSTEM.stats();
    let probe = std::hint::black_box(vec![0_u8; 4_096]);
    let after = stats_alloc::INSTRUMENTED_SYSTEM.stats();
    let valid = after.allocations > before.allocations
        && after.bytes_allocated >= before.bytes_allocated + probe.len();
    drop(probe);
    require(valid, || {
        "verification-cost-counts 调用方必须装配 stats_alloc 全局分配器".into()
    })
}
