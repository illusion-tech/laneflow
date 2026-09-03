use std::alloc::System;

use laneflow_compiler::{
    CanonicalFrameInput, CompilationOutput, CompilationUnitBuilder, CompileLimitDimension,
    CompileLimits, Compiler, PortableDiffBase, PortableEmissionError, PortableEmissionProvenance,
    SourceModuleHeader, SourceModuleHeaderInput, SyntheticModuleBuilder,
    check_portable_policy_diff, check_portable_policy_sources, emit_portable_candidate,
};
use laneflow_format::{FormatLimits, preflight_object_values};
use laneflow_static_contract::PortableObjectKind;
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

fn output(modules: u32, frames_per_module: u32) -> CompilationOutput {
    let limits = CompileLimits::single_network_1m_v2();
    let mut unit = CompilationUnitBuilder::new(limits.clone());
    for module in 0..modules {
        let namespace = format!("review/module-{module:05}");
        // 文档 key 顺序与模块规范顺序相反，不能按原文档向量直接二分。
        let document = format!("review/document-{:05}", modules - module);
        let header = SourceModuleHeader::new(
            SourceModuleHeaderInput {
                authoring_namespace_id: &namespace,
                source_document_key: &document,
                generator_build_id: "review-resource-test",
                parameters_and_inputs_digest: [1; 32],
                frontend_options_digest: [2; 32],
                random_seed: None,
                provenance: "repository:laneflow",
            },
            &limits,
        )
        .unwrap();
        let mut builder = SyntheticModuleBuilder::new(header, &limits).unwrap();
        for frame in 0..frames_per_module {
            builder
                .add_canonical_frame(CanonicalFrameInput {
                    canonical_frame_key: &format!("frame-{frame:05}"),
                    lane_edge_geometries: &[],
                })
                .unwrap();
        }
        unit.add_synthetic_module(builder.finish().unwrap())
            .unwrap();
    }
    Compiler::new().compile(unit.build().unwrap()).unwrap()
}

// 分配统计在独立 integration test 二进制的唯一测试中执行，避免其他测试线程污染。
#[test]
fn policy_checkers_bound_indexes_and_resolve_many_nonlexicographic_documents() {
    let provenance = PortableEmissionProvenance::try_new("review-resource-test").unwrap();
    let small = CompileLimits::p100_initial_v1();
    const SCRATCH_LIMIT: usize = 304_896;
    for count in [4_000, 8_000] {
        let output = output(1, count);
        let candidate = emit_portable_candidate(
            &output,
            &provenance,
            FormatLimits::HARD,
            PortableDiffBase::Genesis,
        )
        .unwrap();
        let artifact = candidate.canonical_artifact().bytes();
        let region = Region::new(GLOBAL);
        let result = check_portable_policy_diff(
            PortableDiffBase::Genesis,
            artifact,
            candidate.semantic_diff().bytes(),
            FormatLimits::HARD,
            &small,
        );
        let stats = region.change();
        assert_eq!(stats.bytes_allocated, stats.bytes_deallocated);
        if count == 8_000 {
            assert!(matches!(
                result,
                Err(PortableEmissionError::CompileLimitExceeded {
                    dimension: CompileLimitDimension::StageScratchBytes,
                    ..
                })
            ));
            assert_eq!(
                stats.allocations, 0,
                "reject before the first index allocation"
            );
        } else {
            result.unwrap();
            assert!(stats.bytes_allocated > 0 && stats.bytes_allocated <= SCRATCH_LIMIT);
            let base = preflight_object_values(
                artifact,
                PortableObjectKind::CanonicalArtifact,
                FormatLimits::HARD,
            )
            .unwrap();
            let no_op = emit_portable_candidate(
                &output,
                &provenance,
                FormatLimits::HARD,
                PortableDiffBase::Artifact(base),
            )
            .unwrap();
            let region = Region::new(GLOBAL);
            let result = check_portable_policy_diff(
                PortableDiffBase::Artifact(base),
                artifact,
                no_op.semantic_diff().bytes(),
                FormatLimits::HARD,
                &small,
            );
            let stats = region.change();
            assert!(
                matches!(
                    result,
                    Err(PortableEmissionError::CompileLimitExceeded {
                        dimension: CompileLimitDimension::StageScratchBytes,
                        ..
                    })
                ),
                "two roots must share the limit"
            );
            assert!(stats.bytes_allocated > 0 && stats.bytes_allocated <= SCRATCH_LIMIT);
            assert_eq!(stats.bytes_allocated, stats.bytes_deallocated);
            check_portable_policy_diff(
                PortableDiffBase::Artifact(base),
                artifact,
                no_op.semantic_diff().bytes(),
                FormatLimits::HARD,
                &CompileLimits::single_network_1m_v2(),
            )
            .unwrap();
        }
        check_portable_policy_diff(
            PortableDiffBase::Genesis,
            artifact,
            candidate.semantic_diff().bytes(),
            FormatLimits::HARD,
            &CompileLimits::single_network_1m_v2(),
        )
        .unwrap();
    }
    let many_modules = output(1_024, 1);
    let candidate = emit_portable_candidate(
        &many_modules,
        &provenance,
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .unwrap();
    assert_eq!(
        many_modules.source_map_input().source_modules().len(),
        1_024
    );
    check_portable_policy_sources(
        candidate.canonical_artifact().bytes(),
        many_modules.source_map_input(),
        candidate.source_map().bytes(),
        FormatLimits::HARD,
        &CompileLimits::single_network_1m_v2(),
    )
    .unwrap();
}
