//! #1441 正式常规档的实际计数、拒绝边界与失败复用验证。

use crate as compiler_api;
use crate::{
    CompileLimitDimension as D, CompileProfile, Compiler, DiagnosticBundle, DiagnosticPayload,
};

mod capacity;
mod workload;
use workload::{
    CANDIDATE_OPERATION_BYTES, SHAPES, WorkloadShape, assemble_mixed_network, build_mixed_network,
    build_synthetic_module,
};

// 登记工作负载的逐维来源包络与达到该值的负载。
const ADMISSION_BOUNDARIES: [(D, u32, usize); 16] = [
    (D::ModuleCount, 18, 4),
    (D::SourceDocumentCount, 18, 4),
    (D::ImportEdgeCount, 17, 4),
    (D::SourceBytesTotal, 8_729_303, 10),
    (D::DeclarationCount, 10_000, 0),
    (D::TypedAstRecordCount, 381_284, 10),
    (D::ReferenceCount, 77_602, 10),
    (D::RelationOccurrenceCount, 105_967, 10),
    (D::IdentityFieldOccurrenceCount, 26_333, 0),
    (D::SymbolCount, 10_000, 0),
    (D::StringItemCount, 164_951, 10),
    (D::TotalStringBytes, 4_815_555, 10),
    (D::StableEntityCount, 10_000, 0),
    (D::ManeuverGateCount, 1_784, 3),
    (D::WaitingZoneCount, 399, 0),
    (D::GeometryPointCount, 128_511, 6),
];
const SOURCE_BYTES_PER_MODULE: u32 = 6_802_754;
const HIR_RECORDS: u32 = 535_412;
const MIR_RECORDS: u32 = 395_015;
const LIR_RECORDS: u32 = 396_669;

fn candidate_limits() -> CompileProfile {
    CompileProfile::single_network_10k_stable_entities()
}

fn assert_limit(diagnostics: &DiagnosticBundle, dimension: D) {
    assert!(
        diagnostics.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic.payload(),
            DiagnosticPayload::CompileLimitExceeded { dimension: actual, limit, observed }
                if *actual == dimension && observed > limit
        )),
        "{dimension:?}: {diagnostics}"
    );
}

#[test]
fn candidate_source_admission_and_stage_counts_match_workload_envelope() {
    let mut envelope = [0_u64; ADMISSION_BOUNDARIES.len()];
    let mut stage_envelope = [0_u64; 4];
    let mut compiler = Compiler::new();
    for (name, shape) in SHAPES {
        for disjoint in [false, true] {
            let shape = WorkloadShape { disjoint, ..shape };
            let builder = assemble_mixed_network(shape, false, candidate_limits())
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            for (index, (dimension, _, _)) in ADMISSION_BOUNDARIES.iter().enumerate() {
                envelope[index] = envelope[index].max(builder.already_admitted(*dimension));
            }
            let unit = builder.build().unwrap();
            let per_module = unit
                .source_document_descriptors()
                .map(|doc| u64::from(doc.source_record_byte_len()))
                .max()
                .unwrap();
            let hir = crate::hir::build_hir(&unit).unwrap();
            let mir = crate::mir::lower_to_mir(&unit, &hir).unwrap();
            let counts = [per_module, hir.hir_record_count, mir.mir_record_count];
            drop((hir, mir));
            let output = compiler
                .compile(unit)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            println!(
                "regular-profile work case={name} disjoint={disjoint} units={}",
                output.metrics().policy_validation_work_units()
            );
            for (index, count) in counts
                .into_iter()
                .chain([output.metrics().lir_record_count()])
                .enumerate()
            {
                stage_envelope[index] = stage_envelope[index].max(count);
            }
            assert!(output.metrics().compiler_controlled_peak_bytes() <= CANDIDATE_OPERATION_BYTES);
            assert!(compiler.retained_capacity_bytes() <= 16 * 1024 * 1024);
        }
    }
    assert_eq!(
        envelope,
        ADMISSION_BOUNDARIES.map(|(_, value, _)| u64::from(value))
    );
    assert_eq!(
        stage_envelope,
        [
            SOURCE_BYTES_PER_MODULE,
            HIR_RECORDS,
            MIR_RECORDS,
            LIR_RECORDS
        ]
        .map(u64::from)
    );
}

#[test]
fn candidate_admission_dimensions_accept_exact_boundary_and_reject_one_less() {
    let mut compiler = Compiler::new();
    for (dimension, limit, case) in ADMISSION_BOUNDARIES {
        let shape = WorkloadShape {
            disjoint: true,
            ..SHAPES[case].1
        };
        let accepted = assemble_mixed_network(shape, false, candidate_limits()).unwrap();
        assert_eq!(
            accepted.already_admitted(dimension),
            u64::from(limit),
            "{dimension:?}"
        );
        assert!(accepted.build().is_ok());
        compiler.compile(build_mixed_network(shape, false)).unwrap();
        let narrow = candidate_limits().with_test_admission_limit(dimension, limit - 1);
        let error = assemble_mixed_network(shape, false, narrow)
            .err()
            .expect("boundary must reject");
        assert_limit(&error, dimension);
    }
}

#[test]
fn source_module_bytes_and_single_string_have_distinct_boundaries() {
    let shape = WorkloadShape {
        disjoint: true,
        ..SHAPES[5].1
    };
    let module = build_synthetic_module(&candidate_limits(), shape, 0, false).unwrap();
    assert_eq!(
        module
            .source_documents()
            .next()
            .unwrap()
            .source_record_byte_len(),
        SOURCE_BYTES_PER_MODULE
    );
    let narrow =
        candidate_limits().with_test_source_byte_limits(SOURCE_BYTES_PER_MODULE - 1, 8_729_303);
    let error = build_synthetic_module(&narrow, shape, 0, false)
        .err()
        .expect("source byte boundary");
    assert_limit(&error, D::SourceBytesPerModule);

    let header = |document: &str| {
        crate::SourceModuleHeader::new(
            crate::SourceModuleHeaderInput {
                authoring_namespace_id: "candidate/string",
                source_document_key: document,
                generator_build_id: "candidate",
                parameters_and_inputs_digest: [0; 32],
                frontend_options_digest: [0; 32],
                random_seed: None,
                provenance: "candidate",
            },
            &candidate_limits(),
        )
    };
    assert!(header(&"d".repeat(1_024)).is_ok());
    let error = header(&"d".repeat(1_025)).err().expect("string boundary");
    assert!(matches!(
        error.diagnostics()[0].payload(),
        DiagnosticPayload::InvalidSourceHeaderField {
            field: crate::SourceHeaderField::SourceDocumentKey,
            violation: crate::SourceTextViolation::TooLong {
                limit: 1_024,
                observed: 1_025
            },
        }
    ));
}

#[test]
fn candidate_stage_record_limits_reject_without_poisoning_compiler() {
    let shape = SHAPES[10].1;
    let mut compiler = Compiler::new();
    // 使用完整常规档，只收紧当前阶段的记录维度。
    let stage_limits = candidate_limits;
    for (dimension, limits) in [
        (
            D::HirRecordCount,
            stage_limits().with_test_pipeline_limits(
                HIR_RECORDS - 1,
                MIR_RECORDS,
                CANDIDATE_OPERATION_BYTES as u32,
            ),
        ),
        (
            D::MirRecordCount,
            stage_limits().with_test_pipeline_limits(
                HIR_RECORDS,
                MIR_RECORDS - 1,
                CANDIDATE_OPERATION_BYTES as u32,
            ),
        ),
        (
            D::LirRecordCount,
            stage_limits().with_test_lir_limits(LIR_RECORDS - 1, CANDIDATE_OPERATION_BYTES as u32),
        ),
    ] {
        let unit = assemble_mixed_network(shape, false, limits)
            .unwrap()
            .build()
            .unwrap();
        let error = compiler.compile(unit).err().expect("stage boundary");
        assert_limit(&error, dimension);
        let unit = assemble_mixed_network(shape, false, stage_limits())
            .unwrap()
            .build()
            .unwrap();
        assert!(compiler.compile(unit).is_ok());
        assert!(compiler.retained_capacity_bytes() <= 16 * 1024 * 1024);
    }
}

#[test]
fn policy_work_boundary_is_independent_of_source_relations_and_allows_reuse() {
    let mut compiler = Compiler::new();
    for shape in [SHAPES[1].1, SHAPES[10].1] {
        let build = |limits| {
            assemble_mixed_network(shape, false, limits)
                .unwrap()
                .build()
                .unwrap()
        };
        let baseline = compiler.compile(build(candidate_limits())).unwrap();
        let used = baseline.metrics().policy_validation_work_units();
        assert!(used > candidate_limits().value(D::RelationOccurrenceCount));
        let exact = candidate_limits().with_test_policy_work_limit(u32::try_from(used).unwrap());
        let output = compiler.compile(build(exact.clone())).unwrap();
        assert_eq!(output.metrics(), baseline.metrics());
        let error = compiler
            .compile(build(
                candidate_limits().with_test_policy_work_limit(u32::try_from(used - 1).unwrap()),
            ))
            .err()
            .expect("policy work boundary");
        assert!(error.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic.payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: D::PolicyValidationWorkUnits,
                limit,
                observed
            } if *limit == used - 1 && *observed == used
        )));
        let retry = compiler.compile(build(exact)).unwrap();
        assert_eq!(retry.metrics(), baseline.metrics());
        let provenance = crate::PortableEmissionProvenance::try_new("policy-work-budget").unwrap();
        let emit = |output: &crate::CompilationOutput| {
            crate::emit_portable_candidate(
                output,
                &provenance,
                laneflow_format::FormatLimits::HARD,
                crate::PortableDiffBase::Genesis,
            )
            .unwrap()
        };
        let a = emit(&baseline);
        let b = emit(&retry);
        assert_eq!(
            a.canonical_artifact().bytes(),
            b.canonical_artifact().bytes()
        );
        assert_eq!(a.source_map().bytes(), b.source_map().bytes());
        assert_eq!(a.semantic_diff().bytes(), b.semantic_diff().bytes());
    }
}

#[test]
fn candidate_diagnostics_retain_sixteen_in_canonical_order_and_allow_reuse() {
    let mut compiler = Compiler::new();
    for count in [16, 17] {
        let limits = candidate_limits();
        let header = crate::SourceModuleHeader::new(
            crate::SourceModuleHeaderInput {
                authoring_namespace_id: "candidate/diagnostics",
                source_document_key: "diagnostics",
                generator_build_id: "candidate",
                parameters_and_inputs_digest: [0; 32],
                frontend_options_digest: [0; 32],
                random_seed: None,
                provenance: "candidate",
            },
            &limits,
        )
        .unwrap();
        let mut module = crate::SyntheticModuleBuilder::new(header, &limits).unwrap();
        for index in (0..count).rev() {
            module
                .add_lane_edge(crate::LaneEdgeInput {
                    lane_edge_key: &format!("edge-{index:02}"),
                    length_meters: 10.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &[crate::LaneEdgeReference::local("missing")],
                })
                .unwrap();
        }
        let mut unit = crate::CompilationUnitBuilder::new(limits);
        unit.add_synthetic_module(module.finish().unwrap()).unwrap();
        let error = compiler.compile(unit.build().unwrap()).err().unwrap();
        assert_eq!(error.diagnostics().len(), 16);
        assert_eq!(error.diagnostics_truncated(), count > 16);
        assert_eq!(
            error
                .diagnostics()
                .iter()
                .map(|diagnostic| diagnostic.stable_key().unwrap())
                .collect::<Vec<_>>(),
            (0..16)
                .map(|index| format!("edge-{index:02}"))
                .collect::<Vec<_>>()
        );
        assert!(error.diagnostics().iter().all(|diagnostic| {
            diagnostic.code() == crate::DiagnosticCode::UnknownReferenceTarget
        }));
        assert!(
            compiler
                .compile(build_mixed_network(SHAPES[0].1, false))
                .is_ok()
        );
    }
}

#[test]
fn combined_policy_diff_fits_256_mib_and_budget_failure_allows_reuse() {
    use crate::{
        PortableDiffBase, PortableEmissionError, PortableEmissionProvenance,
        check_portable_candidate, emit_portable_candidate, emit_portable_candidate_to_staging,
    };
    use laneflow_format::{FormatLimits, preflight_object_values};
    use laneflow_static_contract::PortableObjectKind;
    let shape = SHAPES[10].1;
    let limits = candidate_limits;
    let mut compiler = Compiler::new();
    let output = compiler
        .compile(
            assemble_mixed_network(shape, false, limits())
                .unwrap()
                .build()
                .unwrap(),
        )
        .unwrap();
    let mut target = compiler
        .compile(
            assemble_mixed_network(
                WorkloadShape {
                    disjoint: true,
                    ..shape
                },
                false,
                limits(),
            )
            .unwrap()
            .build()
            .unwrap(),
        )
        .unwrap();
    let provenance = PortableEmissionProvenance::try_new("regular-profile-candidate").unwrap();
    let base = emit_portable_candidate(
        &output,
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
    let diff_base = PortableDiffBase::Artifact(base_view);
    let memory =
        emit_portable_candidate(&target, &provenance, FormatLimits::HARD, diff_base).unwrap();
    let peak = memory.resource_report().peak_live_bytes();
    assert!(peak <= CANDIDATE_OPERATION_BYTES);
    let directory = std::env::temp_dir().join(format!(
        "laneflow-regular-policy-budget-{}",
        std::process::id()
    ));
    std::fs::create_dir(&directory).unwrap();
    let file = emit_portable_candidate_to_staging(
        &target,
        &provenance,
        FormatLimits::HARD,
        diff_base,
        &directory,
    )
    .unwrap();
    assert!(file.resource_report().peak_live_bytes() <= CANDIDATE_OPERATION_BYTES);
    assert_eq!(
        file.canonical_artifact().bytes(),
        memory.canonical_artifact().bytes()
    );
    assert_eq!(file.source_map().bytes(), memory.source_map().bytes());
    assert_eq!(file.semantic_diff().bytes(), memory.semantic_diff().bytes());
    check_portable_candidate(file, FormatLimits::HARD).unwrap();
    assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0);
    std::fs::remove_dir(&directory).unwrap();

    // 动态核对当前实现的真实预算边界，不把优化前的 256 MiB 拒绝固化为产品合同。
    target.set_test_compile_limits(limits().with_test_admission_limit(
        D::CompilerControlledLiveBytes,
        u32::try_from(peak - 1).unwrap(),
    ));
    let error = emit_portable_candidate(&target, &provenance, FormatLimits::HARD, diff_base)
        .expect_err("one byte below peak must reject");
    assert!(
        matches!(error, PortableEmissionError::CompileLimitExceeded {
        dimension: D::CompilerControlledLiveBytes, actual, limit
    } if limit == peak - 1 && actual > limit)
    );
    target.set_test_compile_limits(
        limits().with_test_admission_limit(
            D::CompilerControlledLiveBytes,
            u32::try_from(peak).unwrap(),
        ),
    );
    let exact =
        emit_portable_candidate(&target, &provenance, FormatLimits::HARD, diff_base).unwrap();
    assert_eq!(
        exact.canonical_artifact().bytes(),
        memory.canonical_artifact().bytes()
    );
    assert_eq!(exact.source_map().bytes(), memory.source_map().bytes());
    assert_eq!(
        exact.semantic_diff().bytes(),
        memory.semantic_diff().bytes()
    );
    check_portable_candidate(exact, FormatLimits::HARD).unwrap();
    check_portable_candidate(memory, FormatLimits::HARD).unwrap();
    target.set_test_compile_limits(limits());
    let retry = emit_portable_candidate(
        &target,
        &provenance,
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .unwrap();
    check_portable_candidate(retry, FormatLimits::HARD).unwrap();
    check_portable_candidate(base, FormatLimits::HARD).unwrap();
}
