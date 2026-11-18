//! #1441 内存发射的实际分配与受控账本交叉验证；不采集墙钟性能。

use std::alloc::System;

use laneflow_compiler as compiler_api;
use laneflow_compiler::{
    Compiler, PortableDiffBase, PortableEmissionProvenance, emit_portable_candidate,
};
use laneflow_format::{FormatLimits, preflight_object_values};
use laneflow_static_contract::PortableObjectKind;
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

mod workload;
use workload::{CANDIDATE_OPERATION_BYTES, SHAPES, WorkloadShape, build_mixed_network};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

// 独立二进制只保留一个测试，避免其他测试线程污染全局分配统计。
#[test]
fn in_memory_emission_has_no_unaccounted_full_object_buffer_and_releases_storage() {
    let shape = SHAPES[10].1;
    let mut compiler = Compiler::new();
    let base_output = compiler.compile(build_mixed_network(shape, false)).unwrap();
    let output = compiler
        .compile(build_mixed_network(
            WorkloadShape {
                disjoint: true,
                ..shape
            },
            false,
        ))
        .unwrap();
    let provenance = PortableEmissionProvenance::try_new("regular-profile-candidate").unwrap();
    let base = emit_portable_candidate(
        &base_output,
        &provenance,
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .unwrap();
    let base_view = preflight_object_values(
        base.canonical_artifact().bytes(),
        PortableObjectKind::CanonicalArtifact,
        FormatLimits::HARD,
    )
    .unwrap();
    for (operation, diff_base) in [
        ("genesis", PortableDiffBase::Genesis),
        ("disjoint", PortableDiffBase::Artifact(base_view)),
    ] {
        let region = Region::new(GLOBAL);
        let candidate =
            emit_portable_candidate(&output, &provenance, FormatLimits::HARD, diff_base).unwrap();
        let report = candidate.resource_report();
        let stats = region.change();
        drop(candidate);
        let released = region.change();
        let controlled_requests = report.released_bytes() + report.transferred_bytes();
        println!(
            "regular-profile-allocations operation={operation} allocations={} reallocations={} allocated_bytes={} controlled_requests={controlled_requests} retained_bytes={} emission_peak={}",
            stats.allocations,
            stats.reallocations,
            stats.bytes_allocated,
            stats.bytes_allocated - stats.bytes_deallocated,
            report.peak_live_bytes(),
        );
        // 账本控制结构的固定开销留有 64 KiB；未计账的整份编码缓冲远超过此界。
        assert!(
            (stats.bytes_allocated as u64).abs_diff(controlled_requests) <= 64 * 1024,
            "allocator and ledger request totals"
        );
        assert_eq!(
            (stats.bytes_allocated - stats.bytes_deallocated) as u64,
            report.transferred_bytes(),
            "returned backing remains accounted"
        );
        assert_eq!(released.bytes_allocated, released.bytes_deallocated);
        assert!(report.peak_live_bytes() <= CANDIDATE_OPERATION_BYTES);
    }
}
