//! 最终 LIR、公开消费入口与差异 base 的独立容量边界。

use super::*;
use crate::compiler::portable_fixture_tests::full_spatial_portable_fixture_unit;
use crate::{
    CanonicalInputSide, CompilationOutput, CompilationUnitBuilder, PortableDiffBase,
    PortableEmissionError, PortableEmissionProvenance, PortablePublicationCandidate,
    check_portable_policy_diff, check_portable_policy_sources, emit_portable_candidate,
    emit_portable_candidate_to_staging,
};
use laneflow_format::{FormatLimits, preflight_object_values};
use laneflow_static_contract::{CanonicalNetworkDimension, PortableObjectKind};

fn compile_fixture(
    compiler: &mut Compiler,
    profile: CompileProfile,
) -> Result<CompilationOutput, DiagnosticBundle> {
    let mut unit = full_spatial_portable_fixture_unit();
    unit.limits = profile;
    compiler.compile(unit)
}

fn provenance() -> PortableEmissionProvenance {
    PortableEmissionProvenance::try_new("capacity-boundaries").unwrap()
}

fn emit(output: &CompilationOutput, base: PortableDiffBase<'_>) -> PortablePublicationCandidate {
    emit_portable_candidate(output, &provenance(), FormatLimits::HARD, base).unwrap()
}

fn assert_capacity(
    error: PortableEmissionError,
    side: CanonicalInputSide,
    dimension: CanonicalNetworkDimension,
    observed: u64,
) {
    assert!(
        matches!(error, PortableEmissionError::CanonicalCapacityExceeded { side: actual_side, violation }
        if actual_side == side && violation.dimension == dimension && violation.observed == observed && violation.limit == observed - 1),
        "{error:?}"
    );
}

#[test]
fn compilation_capacity_boundaries_reject_and_allow_compiler_reuse() {
    let mut compiler = Compiler::new();
    let output = compile_fixture(&mut compiler, candidate_limits()).unwrap();
    let usage = output.lir().canonical_network_usage();
    let expected = emit(&output, PortableDiffBase::Genesis);
    for dimension in CanonicalNetworkDimension::ALL {
        let count = usage.get(dimension);
        assert!(count > 0);
        let exact = candidate_limits().with_test_canonical_limit(dimension, count);
        let output = compile_fixture(&mut compiler, exact).unwrap();
        let actual = emit(&output, PortableDiffBase::Genesis);
        assert_eq!(
            actual.canonical_artifact().bytes(),
            expected.canonical_artifact().bytes()
        );
        assert_eq!(actual.source_map().bytes(), expected.source_map().bytes());
        assert_eq!(
            actual.semantic_diff().bytes(),
            expected.semantic_diff().bytes()
        );
        let error = compile_fixture(
            &mut compiler,
            candidate_limits().with_test_canonical_limit(dimension, count - 1),
        )
        .err()
        .unwrap();
        assert_limit(&error, dimension.into());
        assert!(compile_fixture(&mut compiler, candidate_limits()).is_ok());
    }
}

#[test]
fn public_checks_admit_target_before_any_index_budget() {
    let mut compiler = Compiler::new();
    let output = compile_fixture(&mut compiler, candidate_limits()).unwrap();
    let usage = output.lir().canonical_network_usage();
    let candidate = emit(&output, PortableDiffBase::Genesis);
    for dimension in CanonicalNetworkDimension::ALL {
        let count = usage.get(dimension);
        let exact = candidate_limits().with_test_canonical_limit(dimension, count);
        check_portable_policy_sources(
            candidate.canonical_artifact().bytes(),
            output.source_map_input(),
            candidate.source_map().bytes(),
            FormatLimits::HARD,
            exact.consumer_policy(),
        )
        .unwrap();
        check_portable_policy_diff(
            PortableDiffBase::Genesis,
            candidate.canonical_artifact().bytes(),
            candidate.semantic_diff().bytes(),
            FormatLimits::HARD,
            exact.consumer_policy(),
        )
        .unwrap();
        let small = candidate_limits()
            .with_test_canonical_limit(dimension, count - 1)
            .with_test_admission_limit(D::CompilerControlledLiveBytes, 0);
        let error = check_portable_policy_sources(
            candidate.canonical_artifact().bytes(),
            output.source_map_input(),
            candidate.source_map().bytes(),
            FormatLimits::HARD,
            small.consumer_policy(),
        )
        .unwrap_err();
        assert_capacity(error, CanonicalInputSide::Target, dimension, count);
        let error = check_portable_policy_diff(
            PortableDiffBase::Genesis,
            candidate.canonical_artifact().bytes(),
            candidate.semantic_diff().bytes(),
            FormatLimits::HARD,
            small.consumer_policy(),
        )
        .unwrap_err();
        assert_capacity(error, CanonicalInputSide::Target, dimension, count);
        check_portable_policy_diff(
            PortableDiffBase::Genesis,
            candidate.canonical_artifact().bytes(),
            candidate.semantic_diff().bytes(),
            FormatLimits::HARD,
            candidate_limits().consumer_policy(),
        )
        .unwrap();
    }
}

#[test]
fn diff_base_capacity_is_checked_independently_before_allocation_and_staging() {
    let mut compiler = Compiler::new();
    let output = compile_fixture(&mut compiler, candidate_limits()).unwrap();
    let usage = output.lir().canonical_network_usage();
    let base = emit(&output, PortableDiffBase::Genesis);
    let base_view = preflight_object_values(
        base.canonical_artifact().bytes(),
        PortableObjectKind::CanonicalArtifact,
        FormatLimits::HARD,
    )
    .unwrap();
    let base_arg = PortableDiffBase::Artifact(base_view);
    let mut target = compiler
        .compile(
            CompilationUnitBuilder::new(candidate_limits())
                .build()
                .unwrap(),
        )
        .unwrap();
    let expected = emit(&target, base_arg);
    let directory =
        std::env::temp_dir().join(format!("laneflow-capacity-base-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    for dimension in CanonicalNetworkDimension::ALL {
        let count = usage.get(dimension);
        let narrow = candidate_limits()
            .with_test_canonical_limit(dimension, count - 1)
            .with_test_admission_limit(D::CompilerControlledLiveBytes, 0);
        let error = check_portable_policy_diff(
            base_arg,
            expected.canonical_artifact().bytes(),
            expected.semantic_diff().bytes(),
            FormatLimits::HARD,
            narrow.consumer_policy(),
        )
        .unwrap_err();
        assert_capacity(error, CanonicalInputSide::Base, dimension, count);
        target.set_test_compile_limits(narrow);
        assert_capacity(
            emit_portable_candidate(&target, &provenance(), FormatLimits::HARD, base_arg)
                .unwrap_err(),
            CanonicalInputSide::Base,
            dimension,
            count,
        );
        assert_capacity(
            emit_portable_candidate_to_staging(
                &target,
                &provenance(),
                FormatLimits::HARD,
                base_arg,
                &directory,
            )
            .unwrap_err(),
            CanonicalInputSide::Base,
            dimension,
            count,
        );
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0);
        target.set_test_compile_limits(candidate_limits());
        let retry = emit(&target, base_arg);
        assert_eq!(
            retry.canonical_artifact().bytes(),
            expected.canonical_artifact().bytes()
        );
        assert_eq!(retry.source_map().bytes(), expected.source_map().bytes());
        assert_eq!(
            retry.semantic_diff().bytes(),
            expected.semantic_diff().bytes()
        );
        check_portable_policy_diff(
            base_arg,
            retry.canonical_artifact().bytes(),
            retry.semantic_diff().bytes(),
            FormatLimits::HARD,
            candidate_limits().consumer_policy(),
        )
        .unwrap();
    }
    std::fs::remove_dir(&directory).unwrap();
}

#[test]
fn longest_source_metadata_completes_genesis_and_diff_under_the_regular_profile() {
    let profile = candidate_limits();
    let text = "a".repeat(profile.max_single_string_bytes() as usize);
    let header = crate::SourceModuleHeader::new(
        crate::SourceModuleHeaderInput {
            authoring_namespace_id: "capacity/long-metadata",
            source_document_key: &text,
            generator_build_id: &text,
            parameters_and_inputs_digest: [0; 32],
            frontend_options_digest: [0; 32],
            random_seed: None,
            provenance: &text,
        },
        &profile,
    )
    .unwrap();
    let mut module = crate::SyntheticModuleBuilder::new(header, &profile).unwrap();
    module
        .add_lane_edge(crate::LaneEdgeInput {
            lane_edge_key: "edge",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap();
    let mut unit = CompilationUnitBuilder::new(profile.clone());
    unit.add_synthetic_module(module.finish().unwrap()).unwrap();
    let output = Compiler::new().compile(unit.build().unwrap()).unwrap();
    let genesis = emit(&output, PortableDiffBase::Genesis);
    let base = preflight_object_values(
        genesis.canonical_artifact().bytes(),
        PortableObjectKind::CanonicalArtifact,
        FormatLimits::HARD,
    )
    .unwrap();
    let diff = emit(&output, PortableDiffBase::Artifact(base));
    // 来源政策在消费者类型中不可见；收紧它不能拒绝外部规范字节。
    let consumer_profile = profile.with_test_admission_limit(D::SourceBytesTotal, 0);
    check_portable_policy_sources(
        diff.canonical_artifact().bytes(),
        output.source_map_input(),
        diff.source_map().bytes(),
        FormatLimits::HARD,
        consumer_profile.consumer_policy(),
    )
    .unwrap();
    check_portable_policy_diff(
        PortableDiffBase::Artifact(base),
        diff.canonical_artifact().bytes(),
        diff.semantic_diff().bytes(),
        FormatLimits::HARD,
        consumer_profile.consumer_policy(),
    )
    .unwrap();
}
