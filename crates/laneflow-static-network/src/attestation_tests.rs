//! 共享路网修订认证测试（#959 切片 A；按 Issue #959 验收标准逐项覆盖）。

#[path = "attestation_matrix_tests.rs"]
mod matrix;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use laneflow_format::{FormatLimitConfig, FormatLimits, check_canonical_network_input};
use laneflow_static_contract::{NetworkRevisionId, Sha256Digest};

use crate::SharedNetworkRevision;
use crate::attestation::test_injection;
use crate::attestation::{
    self, AttemptOutcome, AttestationCause, AttestationEvent, AttestationOutcome,
    AttestationTelemetry, AttestationVerdict, Disposition, FirstFailure, MismatchEvidence,
    MismatchSite, OperationFailureKind, OriginField, RootAttestationExpectation, RootComponent,
    SourceAuditNote, SourceFailureDetail, TelemetryDeliveryError, attest_shared_network_revision,
};
use crate::builder::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    arm_allocation_failure_injection, build_shared_network_revision,
    reset_allocation_failure_injection,
};
use crate::origin::{CanonicalNetworkOrigin, StaticContractVersions};

const MIN_HEADLESS: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfca-variants/min-headless.lfca"
);
const FULL_SPATIAL: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfca"
);
const PROVENANCE_BASE: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfca-variants/provenance-base.lfca"
);
const PROVENANCE_BUILD: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfca-variants/provenance-build.lfca"
);
const POLICY_FULL_SPATIAL: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfca"
);

const BUILD_LIMITS: SharedNetworkBuildLimits =
    SharedNetworkBuildLimits::new(64 * 1024 * 1024, 16 * 1024 * 1024);

fn build(bytes: &[u8], spatial: SpatialBuildOption) -> Arc<SharedNetworkRevision> {
    let input = check_canonical_network_input(bytes, FormatLimits::HARD)
        .expect("checked canonical network input");
    build_shared_network_revision(input, SharedNetworkBuildOptions::new(spatial, BUILD_LIMITS))
        .expect("shared network revision")
}

/// 从「同字节健康修订」的 origin 组装期望值（测试夹具无独立引用文件；
/// 生产路径宿主从 `PublishedLfcaReference` 取值）。
fn expectation_from(origin: &CanonicalNetworkOrigin) -> RootAttestationExpectation {
    RootAttestationExpectation::new(
        origin.network_revision(),
        origin.canonical_artifact_digest(),
        origin.canonical_artifact_byte_length(),
    )
}

fn attest(
    root: &Arc<SharedNetworkRevision>,
    bytes: &[u8],
    expectation: RootAttestationExpectation,
    spatial: SpatialBuildOption,
) -> AttestationOutcome {
    attest_shared_network_revision(
        root,
        bytes,
        expectation,
        FormatLimits::HARD,
        SharedNetworkBuildOptions::new(spatial, BUILD_LIMITS),
        None,
    )
}

fn attest_with_options(
    root: &Arc<SharedNetworkRevision>,
    bytes: &[u8],
    expectation: RootAttestationExpectation,
    format_limits: FormatLimits,
    build_options: SharedNetworkBuildOptions<'_>,
) -> AttestationOutcome {
    attest_shared_network_revision(root, bytes, expectation, format_limits, build_options, None)
}

fn distort(root: &mut Arc<SharedNetworkRevision>, mutate: impl FnOnce(&mut SharedNetworkRevision)) {
    mutate(Arc::get_mut(root).expect("test root arc is unique"));
}

fn concluded(outcome: AttestationOutcome) -> crate::attestation::AttestationReport {
    match outcome {
        AttestationOutcome::Concluded(report) => report,
        AttestationOutcome::OperationFailed(failure) => {
            panic!("expected concluded outcome, got operation failure: {failure:?}")
        }
    }
}

fn flip_revision(revision: NetworkRevisionId) -> NetworkRevisionId {
    let mut bytes = revision.as_digest().as_bytes().to_owned();
    bytes[0] ^= 0xFF;
    NetworkRevisionId::from_digest(Sha256Digest::from_bytes(bytes))
}

#[test]
fn healthy_roots_pass_under_both_spatial_options() {
    for (bytes, spatial) in [
        (FULL_SPATIAL, SpatialBuildOption::RetainAvailable),
        (FULL_SPATIAL, SpatialBuildOption::Omit),
        (MIN_HEADLESS, SpatialBuildOption::RetainAvailable),
        (MIN_HEADLESS, SpatialBuildOption::Omit),
    ] {
        let root = build(bytes, spatial);
        let report = concluded(attest(
            &root,
            bytes,
            expectation_from(root.canonical_origin()),
            spatial,
        ));
        assert_eq!(
            report.verdict,
            AttestationVerdict::Pass,
            "bytes/spatial {bytes:?}"
        );
        assert_eq!(report.cause, None);
        assert_eq!(report.disposition, Disposition::None);
        assert!(report.mismatch.is_none());
        assert!(report.source_audit.is_empty());
    }
}

#[test]
fn retain_option_on_omitted_root_mismatches_indeterminate() {
    let omitted_root = build(FULL_SPATIAL, SpatialBuildOption::Omit);
    let report = concluded(attest(
        &omitted_root,
        FULL_SPATIAL,
        expectation_from(omitted_root.canonical_origin()),
        SpatialBuildOption::RetainAvailable,
    ));
    assert_eq!(report.verdict, AttestationVerdict::Mismatch);
    assert_eq!(report.cause, Some(AttestationCause::Indeterminate));
    assert!(matches!(
        report.mismatch.as_ref().expect("evidence").site,
        MismatchSite::SpatialPresence {
            expected: true,
            actual: false
        }
    ));
    assert_eq!(report.disposition, Disposition::DisableRoot);
}

/// 同修订重发布：恢复前（修订由新字节构建）与在用修订审计（修订与同修订的另一份
/// 引用配对）两种情形都判 `Pass`，字节与引用的 digest / length 差异记来源
/// 审计诊断（设计 §2.2「发布来源的字节身份」；Issue #959 验收）。
#[test]
fn republished_same_revision_passes_with_byte_identity_audit() {
    let base_root = build(PROVENANCE_BASE, SpatialBuildOption::RetainAvailable);
    let build_root = build(PROVENANCE_BUILD, SpatialBuildOption::RetainAvailable);
    assert_eq!(
        base_root.network_revision(),
        build_root.network_revision(),
        "fixture pair must share one revision"
    );
    // 恢复前：修订由重发布（build）字节构建，宿主本次字节与存档引用（base）不同份。
    let report = concluded(attest(
        &build_root,
        PROVENANCE_BUILD,
        expectation_from(base_root.canonical_origin()),
        SpatialBuildOption::RetainAvailable,
    ));
    assert_eq!(report.verdict, AttestationVerdict::Pass);
    assert!(
        report.source_audit.iter().any(|note| matches!(
            note,
            SourceAuditNote::ByteIdentityDiffersFromReference { .. }
        )),
        "字节（build）与引用（base）digest 不同 → 记来源审计诊断"
    );
    // 在用修订审计：修订与活动引用均为 base，宿主本次资产认证拿到重发布字节。
    let report = concluded(attest(
        &base_root,
        PROVENANCE_BUILD,
        expectation_from(base_root.canonical_origin()),
        SpatialBuildOption::RetainAvailable,
    ));
    assert_eq!(report.verdict, AttestationVerdict::Pass);
    assert!(
        report.source_audit.iter().any(|note| matches!(
            note,
            SourceAuditNote::ByteIdentityDiffersFromReference { .. }
        )),
        "在用修订（base 构建）与另一份同修订来源配对仍判 Pass 并记诊断"
    );
    // 修订 digest 记录构建字节，不进入任何比对：base 修订配 build 字节也 Pass。
    let report = concluded(attest(
        &base_root,
        PROVENANCE_BUILD,
        expectation_from(build_root.canonical_origin()),
        SpatialBuildOption::RetainAvailable,
    ));
    assert_eq!(report.verdict, AttestationVerdict::Pass);
    assert!(
        report.source_audit.is_empty(),
        "字节与引用同份（build）无诊断"
    );
}

/// 修订 `origin` 字节身份字段在任何调用时机都不比对（漏检面 §5 第 15 条）：
/// 篡改修订 origin 的 digest 后认证仍 Pass。
#[test]
fn root_origin_byte_identity_distortion_is_not_compared() {
    let mut root = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
    let healthy = *root.canonical_origin();
    let mut flipped = healthy.canonical_artifact_digest().as_bytes().to_owned();
    flipped[0] ^= 0xFF;
    let distorted = CanonicalNetworkOrigin::new(
        Sha256Digest::from_bytes(flipped),
        healthy.canonical_artifact_byte_length(),
        healthy.network_revision(),
        healthy.static_contract_versions(),
    );
    distort(&mut root, |revision| revision.origin = distorted);
    let report = concluded(attest(
        &root,
        FULL_SPATIAL,
        expectation_from(&healthy),
        SpatialBuildOption::RetainAvailable,
    ));
    assert_eq!(report.verdict, AttestationVerdict::Pass);
}

#[test]
fn root_revision_distortion_is_indeterminate() {
    let mut root = build(MIN_HEADLESS, SpatialBuildOption::RetainAvailable);
    let healthy = *root.canonical_origin();
    let distorted = CanonicalNetworkOrigin::new(
        healthy.canonical_artifact_digest(),
        healthy.canonical_artifact_byte_length(),
        flip_revision(healthy.network_revision()),
        healthy.static_contract_versions(),
    );
    distort(&mut root, |revision| revision.origin = distorted);
    let report = concluded(attest(
        &root,
        MIN_HEADLESS,
        expectation_from(&healthy),
        SpatialBuildOption::RetainAvailable,
    ));
    assert_eq!(report.verdict, AttestationVerdict::Mismatch);
    assert_eq!(report.cause, Some(AttestationCause::Indeterminate));
    assert!(matches!(
        report.mismatch.as_ref().expect("evidence").site,
        MismatchSite::OriginField {
            field: OriginField::NetworkRevision,
            ..
        }
    ));
}

#[test]
fn root_contract_version_distortion_is_indeterminate() {
    let mut root = build(MIN_HEADLESS, SpatialBuildOption::RetainAvailable);
    let healthy = *root.canonical_origin();
    let contracts = healthy.static_contract_versions();
    let distorted_contracts = StaticContractVersions::new(
        contracts.canonical_format_version(),
        contracts.identity_encoding_version(),
        contracts.identity_registry_revision(),
        contracts.network_revision_derivation_version() + 1,
        contracts.constraint_contract_version(),
        contracts.static_execution_contract_version(),
    );
    let distorted = CanonicalNetworkOrigin::new(
        healthy.canonical_artifact_digest(),
        healthy.canonical_artifact_byte_length(),
        healthy.network_revision(),
        distorted_contracts,
    );
    distort(&mut root, |revision| revision.origin = distorted);
    let report = concluded(attest(
        &root,
        MIN_HEADLESS,
        expectation_from(&healthy),
        SpatialBuildOption::RetainAvailable,
    ));
    assert_eq!(report.verdict, AttestationVerdict::Mismatch);
    assert_eq!(report.cause, Some(AttestationCause::Indeterminate));
    assert!(matches!(
        report.mismatch.as_ref().expect("evidence").site,
        MismatchSite::OriginField {
            field: OriginField::NetworkRevisionDerivationVersion,
            ref expected,
            ref actual,
        } if expected == "1" && actual == "2"
    ));
}

#[test]
fn root_planning_hints_version_distortion_is_indeterminate() {
    let mut root = build(MIN_HEADLESS, SpatialBuildOption::RetainAvailable);
    let healthy = *root.canonical_origin();
    distort(&mut root, |revision| {
        revision.origin = healthy.test_with_partition_hints_version(
            crate::origin::PARTITION_PLANNING_HINTS_DERIVATION_VERSION + 1,
        );
    });
    let report = concluded(attest(
        &root,
        MIN_HEADLESS,
        expectation_from(&healthy),
        SpatialBuildOption::RetainAvailable,
    ));
    assert_eq!(report.verdict, AttestationVerdict::Mismatch);
    assert_eq!(report.cause, Some(AttestationCause::Indeterminate));
    assert!(matches!(
        report.mismatch.as_ref().expect("evidence").site,
        MismatchSite::OriginField {
            field: OriginField::PartitionPlanningHintsDerivationVersion,
            ..
        }
    ));
}

/// 单张派生表失真（来源通过、比对确认失配）判 `Mismatch` 判因
/// `共享路网修订`，而不是 `Untrusted`。
#[test]
fn derived_table_distortion_is_shared_static_root() {
    let mut root = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
    distort(&mut root, |revision| {
        if let Some(weight) = revision.planning_hints.edge_boundary_weights.first_mut() {
            *weight = weight.wrapping_add(1);
        } else {
            panic!("fixture must carry a non-empty planning hints column");
        }
    });
    let report = concluded(attest(
        &root,
        FULL_SPATIAL,
        expectation_from(root.canonical_origin()),
        SpatialBuildOption::RetainAvailable,
    ));
    assert_eq!(report.verdict, AttestationVerdict::Mismatch);
    assert_eq!(report.cause, Some(AttestationCause::SharedStaticRoot));
    assert_eq!(report.disposition, Disposition::DisableRoot);
    assert!(matches!(
        report.mismatch.as_ref().expect("evidence").site,
        MismatchSite::TableValue {
            component: RootComponent::PlanningHints,
            table: "edge_boundary_weights",
            index: 0,
            ..
        }
    ));
}

#[test]
fn traffic_table_distortion_reports_first_unequal_location() {
    let mut root = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
    let healthy_length = root.traffic().lane_lengths_millimetres().to_vec();
    assert!(
        !healthy_length.is_empty(),
        "fixture must carry lane lengths"
    );
    distort(&mut root, |revision| {
        let column = &mut revision.traffic.lane_lengths_millimetres;
        column[0] = column[0].wrapping_add(1_000);
    });
    let report = concluded(attest(
        &root,
        FULL_SPATIAL,
        expectation_from(root.canonical_origin()),
        SpatialBuildOption::RetainAvailable,
    ));
    assert_eq!(report.verdict, AttestationVerdict::Mismatch);
    assert_eq!(report.cause, Some(AttestationCause::SharedStaticRoot));
    match report.mismatch.expect("evidence").site {
        MismatchSite::TableValue {
            component,
            table,
            entity_kind,
            index,
            field,
            expected,
            actual,
        } => {
            assert_eq!(entity_kind, None);
            assert_eq!(component, RootComponent::Traffic);
            assert_eq!(table, "lane_lengths_millimetres");
            assert_eq!(index, 0);
            assert_eq!(field, "value");
            assert_eq!(expected, healthy_length[0].to_string());
            assert_eq!(actual, healthy_length[0].wrapping_add(1_000).to_string());
        }
        other => panic!("expected table-value evidence, got {other:?}"),
    }
}

/// `+0.0` → `-0.0` 与 NaN 位模式注入都判 `Mismatch`（比较算子按 `to_bits()`，
/// 不得用 `PartialEq` 比较浮点）。
#[test]
fn float_bit_pattern_distortions_mismatch() {
    let mut root = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
    distort(&mut root, |revision| {
        let pose = revision
            .spatial
            .as_mut()
            .expect("spatial")
            .lane_pose
            .as_mut()
            .expect("lane pose");
        let zero_index = pose
            .points
            .iter()
            .position(|point| point.x.to_bits() == 0)
            .expect("fixture must carry a +0.0 sample point");
        pose.points[zero_index].x = f32::from_bits(0x8000_0000); // -0.0
    });
    let report = concluded(attest(
        &root,
        FULL_SPATIAL,
        expectation_from(root.canonical_origin()),
        SpatialBuildOption::RetainAvailable,
    ));
    assert_eq!(report.verdict, AttestationVerdict::Mismatch);
    assert_eq!(report.cause, Some(AttestationCause::SharedStaticRoot));

    let mut root = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
    distort(&mut root, |revision| {
        let pose = revision
            .spatial
            .as_mut()
            .expect("spatial")
            .lane_pose
            .as_mut()
            .expect("lane pose");
        // NaN 位模式注入：与健康数值任意位模式不等。
        pose.arc_lengths_meters[0] = f32::from_bits(0x7FC0_0000);
    });
    let report = concluded(attest(
        &root,
        FULL_SPATIAL,
        expectation_from(root.canonical_origin()),
        SpatialBuildOption::RetainAvailable,
    ));
    assert_eq!(report.verdict, AttestationVerdict::Mismatch);
}

/// 来源字节损坏：reader 内容类拒绝 → `Mismatch` 判因 `来源`（在用修订审计
/// 场景下不停用修订——停用是宿主按判因执行的处置，结论类型即边界）。
#[test]
fn corrupted_source_bytes_are_source_cause() {
    let root = build(MIN_HEADLESS, SpatialBuildOption::RetainAvailable);
    let mut corrupted = MIN_HEADLESS.to_vec();
    let mid = corrupted.len() / 2;
    corrupted[mid] ^= 0x01;
    let report = concluded(attest(
        &root,
        &corrupted,
        expectation_from(root.canonical_origin()),
        SpatialBuildOption::RetainAvailable,
    ));
    assert_eq!(report.verdict, AttestationVerdict::Mismatch);
    assert_eq!(report.cause, Some(AttestationCause::Source));
    assert_eq!(report.disposition, Disposition::IsolateSource);
    assert!(matches!(
        report.mismatch.as_ref().expect("evidence").site,
        MismatchSite::SourceFailure {
            detail: SourceFailureDetail::Reader(_),
        }
    ));
}

/// 绑定引用损坏（引用 revision 与字节不符）：失配判给来源而非修订。
#[test]
fn corrupted_reference_revision_is_source_cause() {
    let root = build(MIN_HEADLESS, SpatialBuildOption::RetainAvailable);
    let healthy = *root.canonical_origin();
    let corrupted_expectation = RootAttestationExpectation::new(
        flip_revision(healthy.network_revision()),
        healthy.canonical_artifact_digest(),
        healthy.canonical_artifact_byte_length(),
    );
    let report = concluded(attest(
        &root,
        MIN_HEADLESS,
        corrupted_expectation,
        SpatialBuildOption::RetainAvailable,
    ));
    assert_eq!(report.verdict, AttestationVerdict::Mismatch);
    assert_eq!(report.cause, Some(AttestationCause::Source));
    assert!(matches!(
        report.mismatch.as_ref().expect("evidence").site,
        MismatchSite::SourceFailure {
            detail: SourceFailureDetail::RevisionCriterion { .. },
        }
    ));
}

/// 来源被替换为另一份自洽来源（revision 不同）：来源完整闭合后修订 origin
/// revision 失配判 `无法判因`（设计 §7）。
#[test]
fn different_revision_source_pairing_is_indeterminate() {
    let root = build(MIN_HEADLESS, SpatialBuildOption::RetainAvailable);
    let other = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
    assert_ne!(
        root.network_revision(),
        other.network_revision(),
        "fixtures must carry distinct revisions"
    );
    let report = concluded(attest(
        &root,
        FULL_SPATIAL,
        expectation_from(other.canonical_origin()),
        SpatialBuildOption::RetainAvailable,
    ));
    assert_eq!(report.verdict, AttestationVerdict::Mismatch);
    assert_eq!(report.cause, Some(AttestationCause::Indeterminate));
    assert!(matches!(
        report.mismatch.as_ref().expect("evidence").site,
        MismatchSite::OriginField {
            field: OriginField::NetworkRevision,
            ..
        }
    ));
}

#[test]
fn reader_limit_exceeded_is_operational_failure() {
    let root = build(MIN_HEADLESS, SpatialBuildOption::RetainAvailable);
    let config = FormatLimitConfig {
        max_object_bytes: 16,
        ..FormatLimitConfig::HARD
    };
    let limits = FormatLimits::try_new(config).expect("valid limits");
    let outcome = attest_with_options(
        &root,
        MIN_HEADLESS,
        expectation_from(root.canonical_origin()),
        limits,
        SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, BUILD_LIMITS),
    );
    match outcome {
        AttestationOutcome::OperationFailed(failure) => {
            assert_eq!(failure.kind, OperationFailureKind::ReaderLimitExceeded);
        }
        other => panic!("expected operation failure, got {other:?}"),
    }
}

#[test]
fn builder_budget_exceeded_is_operational_failure() {
    let root = build(MIN_HEADLESS, SpatialBuildOption::RetainAvailable);
    let tiny = SharedNetworkBuildLimits::new(1, 1);
    let outcome = attest_with_options(
        &root,
        MIN_HEADLESS,
        expectation_from(root.canonical_origin()),
        FormatLimits::HARD,
        SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, tiny),
    );
    match outcome {
        AttestationOutcome::OperationFailed(failure) => {
            assert_eq!(failure.kind, OperationFailureKind::BuilderBudgetExceeded);
        }
        other => panic!("expected operation failure, got {other:?}"),
    }
}

#[test]
fn cancellation_is_operational_failure() {
    let root = build(MIN_HEADLESS, SpatialBuildOption::RetainAvailable);
    let cancelled = AtomicBool::new(true);
    let options = SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, BUILD_LIMITS)
        .with_cancellation(&cancelled);
    let outcome = attest_with_options(
        &root,
        MIN_HEADLESS,
        expectation_from(root.canonical_origin()),
        FormatLimits::HARD,
        options,
    );
    match outcome {
        AttestationOutcome::OperationFailed(failure) => {
            assert_eq!(failure.kind, OperationFailureKind::Cancelled);
        }
        other => panic!("expected operation failure, got {other:?}"),
    }
}

#[test]
fn allocation_failure_is_operational_failure() {
    let root = build(MIN_HEADLESS, SpatialBuildOption::RetainAvailable);
    arm_allocation_failure_injection(1);
    let outcome = attest(
        &root,
        MIN_HEADLESS,
        expectation_from(root.canonical_origin()),
        SpatialBuildOption::RetainAvailable,
    );
    reset_allocation_failure_injection();
    match outcome {
        AttestationOutcome::OperationFailed(failure) => {
            assert_eq!(failure.kind, OperationFailureKind::BuilderAllocationFailure);
        }
        other => panic!("expected operation failure, got {other:?}"),
    }
}

/// 来源准备阶段 panic 按来源失败归类：复算不再 panic 时完整通过 →
/// `Pass`，首次失配记为来源审计诊断（在用修订不停用）。
#[test]
fn source_preparation_panic_recovers_to_pass_with_audit() {
    test_injection::arm_source_preparation_panic_once();
    let root = build(MIN_HEADLESS, SpatialBuildOption::RetainAvailable);
    let outcome = attest_shared_network_revision(
        &root,
        MIN_HEADLESS,
        expectation_from(root.canonical_origin()),
        FormatLimits::HARD,
        SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, BUILD_LIMITS),
        None,
    );
    test_injection::reset();
    let report = concluded(outcome);
    assert_eq!(report.verdict, AttestationVerdict::Pass);
    assert!(matches!(
        report.source_audit.first(),
        Some(SourceAuditNote::TransientSourceFailure { .. })
    ));
}

/// 来源准备阶段 panic 复算仍是同一 panic 证据 → `Mismatch` 判因 `来源`。
#[test]
fn source_preparation_panic_recompute_same_evidence_mismatches() {
    test_injection::arm_source_preparation_panic();
    let root = build(MIN_HEADLESS, SpatialBuildOption::RetainAvailable);
    let outcome = attest_shared_network_revision(
        &root,
        MIN_HEADLESS,
        expectation_from(root.canonical_origin()),
        FormatLimits::HARD,
        SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, BUILD_LIMITS),
        None,
    );
    test_injection::reset();
    let report = concluded(outcome);
    assert_eq!(report.verdict, AttestationVerdict::Mismatch);
    assert_eq!(report.cause, Some(AttestationCause::Source));
    assert!(matches!(
        report.mismatch.as_ref().expect("evidence").site,
        MismatchSite::SourceFailure {
            detail: SourceFailureDetail::Panic(ref payload),
        } if payload == "injected source-preparation panic"
    ));
}

/// 修订比较阶段 panic 转为 `Untrusted`，不向宿主传播。
#[test]
fn root_comparison_panic_is_untrusted() {
    test_injection::arm_root_comparison_panic();
    let root = build(MIN_HEADLESS, SpatialBuildOption::RetainAvailable);
    let outcome = attest_shared_network_revision(
        &root,
        MIN_HEADLESS,
        expectation_from(root.canonical_origin()),
        FormatLimits::HARD,
        SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, BUILD_LIMITS),
        None,
    );
    test_injection::reset();
    let report = concluded(outcome);
    assert_eq!(report.verdict, AttestationVerdict::Untrusted);
    assert_eq!(report.cause, Some(AttestationCause::Indeterminate));
    assert!(matches!(
        report.mismatch.as_ref().expect("evidence").site,
        MismatchSite::Panicked { ref payload } if payload == "injected root-comparison panic"
    ));
    assert_eq!(report.disposition, Disposition::DisableRoot);
}

/// §2.2 复算归类表 · 来源侧（穷举：同证据 / 证据不同 / 复算通过 / 转修订侧 / 未完成）。
#[test]
fn classify_recompute_table_source_side() {
    let reader_failure = |error| {
        FirstFailure::Source(MismatchEvidence {
            stage: attestation::AttestationStage::SourcePreparation,
            site: MismatchSite::SourceFailure {
                detail: SourceFailureDetail::Reader(error),
            },
        })
    };
    let mismatch_a =
        reader_failure(laneflow_format::CanonicalNetworkInputError::NetworkRevisionMismatch);
    let mismatch_b =
        reader_failure(laneflow_format::CanonicalNetworkInputError::ArithmeticOverflow);

    // 同一证据 → Mismatch 判因来源。
    let report = concluded(attestation::classify_confirmed_failure(
        mismatch_a.clone(),
        AttemptOutcome::Failed(mismatch_a.clone()),
        Vec::new(),
    ));
    assert_eq!(report.verdict, AttestationVerdict::Mismatch);
    assert_eq!(report.cause, Some(AttestationCause::Source));
    assert_eq!(report.disposition, Disposition::IsolateSource);

    // 证据不同 → 操作失败（来源证据不稳定），两次证据入诊断。
    let outcome = attestation::classify_confirmed_failure(
        mismatch_a.clone(),
        AttemptOutcome::Failed(mismatch_b.clone()),
        Vec::new(),
    );
    match outcome {
        AttestationOutcome::OperationFailed(failure) => {
            assert_eq!(failure.kind, OperationFailureKind::SourceEvidenceUnstable);
        }
        other => panic!("expected operation failure, got {other:?}"),
    }

    // 复算完整通过 → Pass，首次失配记为来源审计诊断。
    let report = concluded(attestation::classify_confirmed_failure(
        mismatch_a.clone(),
        AttemptOutcome::Concluded,
        Vec::new(),
    ));
    assert_eq!(report.verdict, AttestationVerdict::Pass);
    assert!(matches!(
        report.source_audit.first(),
        Some(SourceAuditNote::TransientSourceFailure { .. })
    ));

    // 复算改在修订比较阶段失配 → Untrusted（出现未确认的修订侧证据）。
    let root_side = FirstFailure::RootCompare(MismatchEvidence {
        stage: attestation::AttestationStage::RootComparison,
        site: MismatchSite::TableValue {
            component: RootComponent::Traffic,
            table: "successors",
            entity_kind: None,
            index: 0,
            field: "value",
            expected: "1".to_string(),
            actual: "2".to_string(),
        },
    });
    let report = concluded(attestation::classify_confirmed_failure(
        mismatch_a,
        AttemptOutcome::Failed(root_side),
        Vec::new(),
    ));
    assert_eq!(report.verdict, AttestationVerdict::Untrusted);
    assert_eq!(report.cause, Some(AttestationCause::Indeterminate));

    // 复算未完成 → 操作失败（首次证据入诊断）。
    let outcome = attestation::classify_confirmed_failure(
        mismatch_b,
        AttemptOutcome::OperationFailed(crate::attestation::OperationFailure {
            kind: OperationFailureKind::Cancelled,
            detail: "cancelled".to_string(),
            source_audit: Vec::new(),
            telemetry_dropped: 0,
        }),
        Vec::new(),
    );
    match outcome {
        AttestationOutcome::OperationFailed(failure) => {
            assert_eq!(failure.kind, OperationFailureKind::RecomputeIncomplete);
        }
        other => panic!("expected operation failure, got {other:?}"),
    }
}

/// §2.2 复算归类表 · 修订侧（同证据 → Mismatch 按判因；任一项不同（含复算
/// 通过）或复算未完成 → Untrusted 且保留首次证据；两次失配位置不同的情形
/// 即「证据不同」分支）。
#[test]
fn classify_recompute_table_root_side() {
    let table_evidence = |index: u32| {
        FirstFailure::RootCompare(MismatchEvidence {
            stage: attestation::AttestationStage::RootComparison,
            site: MismatchSite::TableValue {
                component: RootComponent::PlanningHints,
                table: "edge_boundary_weights",
                entity_kind: None,
                index,
                field: "value",
                expected: "1".to_string(),
                actual: "2".to_string(),
            },
        })
    };

    // 同一证据 → Mismatch 判因共享路网修订。
    let report = concluded(attestation::classify_confirmed_failure(
        table_evidence(3),
        AttemptOutcome::Failed(table_evidence(3)),
        Vec::new(),
    ));
    assert_eq!(report.verdict, AttestationVerdict::Mismatch);
    assert_eq!(report.cause, Some(AttestationCause::SharedStaticRoot));
    assert_eq!(report.disposition, Disposition::DisableRoot);

    // 两次失配位置不同（证据不同）→ Untrusted，保留首次证据。
    let report = concluded(attestation::classify_confirmed_failure(
        table_evidence(3),
        AttemptOutcome::Failed(table_evidence(7)),
        Vec::new(),
    ));
    assert_eq!(report.verdict, AttestationVerdict::Untrusted);
    let evidence = report.mismatch.expect("first evidence retained");
    assert!(matches!(
        evidence.site,
        MismatchSite::TableValue { index: 3, .. }
    ));

    // 复算通过 → Untrusted（修订侧证据不可复现）。
    let report = concluded(attestation::classify_confirmed_failure(
        table_evidence(3),
        AttemptOutcome::Concluded,
        Vec::new(),
    ));
    assert_eq!(report.verdict, AttestationVerdict::Untrusted);

    // 复算未完成 → Untrusted（取消不能撤销修订侧证据）。
    let report = concluded(attestation::classify_confirmed_failure(
        table_evidence(3),
        AttemptOutcome::OperationFailed(crate::attestation::OperationFailure {
            kind: OperationFailureKind::Cancelled,
            detail: "cancelled".to_string(),
            source_audit: Vec::new(),
            telemetry_dropped: 0,
        }),
        Vec::new(),
    ));
    assert_eq!(report.verdict, AttestationVerdict::Untrusted);

    // origin 失配的同证据复算 → Mismatch 判因无法判因。
    let origin_evidence = || {
        FirstFailure::RootCompare(MismatchEvidence {
            stage: attestation::AttestationStage::RootComparison,
            site: MismatchSite::OriginField {
                field: OriginField::NetworkRevision,
                expected: "a".to_string(),
                actual: "b".to_string(),
            },
        })
    };
    let report = concluded(attestation::classify_confirmed_failure(
        origin_evidence(),
        AttemptOutcome::Failed(origin_evidence()),
        Vec::new(),
    ));
    assert_eq!(report.verdict, AttestationVerdict::Mismatch);
    assert_eq!(report.cause, Some(AttestationCause::Indeterminate));
}

/// 不同 NaN payload 必须识别为不同证据：tangent 注入两个仅 payload 不同的
/// NaN，两次认证的失配证据不得相等（复算归类依赖证据判等，证据相等性
/// 不得因 Debug 展示相同而误判）。
#[test]
fn distinct_nan_payloads_produce_distinct_evidence() {
    let first = {
        let mut root = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
        distort(&mut root, |revision| {
            let pose = revision
                .spatial
                .as_mut()
                .expect("spatial")
                .lane_pose
                .as_mut()
                .expect("lane pose");
            assert!(!pose.segments.is_empty(), "fixture must carry segments");
            pose.segments[0].tangent[0] = f32::from_bits(0x7FC0_0000);
        });
        let report = concluded(attest(
            &root,
            FULL_SPATIAL,
            expectation_from(root.canonical_origin()),
            SpatialBuildOption::RetainAvailable,
        ));
        assert_eq!(report.verdict, AttestationVerdict::Mismatch);
        report.mismatch.expect("evidence")
    };
    let second = {
        let mut root = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
        distort(&mut root, |revision| {
            let pose = revision
                .spatial
                .as_mut()
                .expect("spatial")
                .lane_pose
                .as_mut()
                .expect("lane pose");
            pose.segments[0].tangent[0] = f32::from_bits(0x7FC0_0001);
        });
        let report = concluded(attest(
            &root,
            FULL_SPATIAL,
            expectation_from(root.canonical_origin()),
            SpatialBuildOption::RetainAvailable,
        ));
        assert_eq!(report.verdict, AttestationVerdict::Mismatch);
        report.mismatch.expect("evidence")
    };
    assert_ne!(first, second, "不同 NaN payload 的证据必须不同");
}

/// 相同数值失配从一种 EntityKind 移到另一种必须判为不同位置：identity
/// 正向索引按 kind 展开为多张表，kind 进入证据身份。
#[test]
fn same_value_mismatch_across_entity_kinds_is_distinct_evidence() {
    let kinds = {
        let root = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
        let identity = root.identity();
        let non_empty: Vec<_> = laneflow_static_contract::EntityKind::ALL
            .iter()
            .enumerate()
            .filter(|(index, _)| !identity.forward[*index].is_empty())
            .map(|(index, kind)| (index, *kind))
            .collect();
        assert!(
            non_empty.len() >= 2,
            "fixture must carry at least two non-empty identity kinds"
        );
        non_empty
    };
    let (first_index, first_kind) = kinds[0];
    let (second_index, second_kind) = kinds[1];
    let truncate_at = |root: &mut Arc<SharedNetworkRevision>, index: usize| {
        distort(root, |revision| {
            let column = &mut revision.identity.forward[index];
            let mut values = column.to_vec();
            values.truncate(values.len() - 1);
            *column = values.into_boxed_slice();
        });
    };
    let evidence_for = |index: usize| {
        let mut root = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
        truncate_at(&mut root, index);
        let report = concluded(attest(
            &root,
            FULL_SPATIAL,
            expectation_from(root.canonical_origin()),
            SpatialBuildOption::RetainAvailable,
        ));
        assert_eq!(report.verdict, AttestationVerdict::Mismatch);
        report.mismatch.expect("evidence")
    };
    let first = evidence_for(first_index);
    let second = evidence_for(second_index);
    assert_ne!(first, second, "失配移到另一个 kind 的表是不同证据");
    assert!(matches!(
        (&first.site, &second.site),
        (
            MismatchSite::TableLength {
                entity_kind: Some(a),
                ..
            },
            MismatchSite::TableLength {
                entity_kind: Some(b),
                ..
            }
        ) if a == &first_kind && b == &second_kind
    ));
}

/// 来源复算未完成：操作失败结果保留首次证据（SourceRecomputeIncomplete
/// 入 source_audit，不得记为瞬时恢复），遥测仍发出 L3 Retry 处置（不伪造
/// 三值结论）。
#[test]
fn recompute_incomplete_retains_first_evidence_and_emits_retry() {
    let root = build(MIN_HEADLESS, SpatialBuildOption::RetainAvailable);
    test_injection::arm_source_preparation_panic_once();
    arm_allocation_failure_injection(1);
    let mut sink = RecordingSink::default();
    let outcome = attest_shared_network_revision(
        &root,
        MIN_HEADLESS,
        expectation_from(root.canonical_origin()),
        FormatLimits::HARD,
        SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, BUILD_LIMITS),
        Some(&mut sink),
    );
    test_injection::reset();
    reset_allocation_failure_injection();
    match outcome {
        AttestationOutcome::OperationFailed(failure) => {
            assert_eq!(failure.kind, OperationFailureKind::RecomputeIncomplete);
            assert!(
                failure
                    .source_audit
                    .iter()
                    .any(|note| matches!(note, SourceAuditNote::SourceRecomputeIncomplete { .. })),
                "首次来源失配证据以未完成复算标签保留在操作失败结果中"
            );
            assert!(
                !failure
                    .source_audit
                    .iter()
                    .any(|note| matches!(note, SourceAuditNote::TransientSourceFailure { .. })),
                "复算未完成不得记为瞬时恢复（复算没有通过）"
            );
        }
        other => panic!("expected operation failure, got {other:?}"),
    }
    assert!(sink.events.contains(&"disposition"), "L3 Retry 处置照发");
    assert!(
        !sink.events.contains(&"verdict"),
        "操作失败不伪造三值结论事件"
    );
}

/// revision 与字节身份同时不一致：同修订判据失配判来源，字节身份差异
/// 诊断仍被记录（记录先于 revision 提前返回）。
#[test]
fn revision_and_byte_identity_mismatch_both_retained() {
    let root = build(MIN_HEADLESS, SpatialBuildOption::RetainAvailable);
    let healthy = *root.canonical_origin();
    let mut flipped_digest = healthy.canonical_artifact_digest().as_bytes().to_owned();
    flipped_digest[0] ^= 0xFF;
    let mismatched = RootAttestationExpectation::new(
        flip_revision(healthy.network_revision()),
        Sha256Digest::from_bytes(flipped_digest),
        healthy.canonical_artifact_byte_length(),
    );
    let report = concluded(attest(
        &root,
        MIN_HEADLESS,
        mismatched,
        SpatialBuildOption::RetainAvailable,
    ));
    assert_eq!(report.verdict, AttestationVerdict::Mismatch);
    assert_eq!(report.cause, Some(AttestationCause::Source));
    assert!(matches!(
        report.mismatch.as_ref().expect("evidence").site,
        MismatchSite::SourceFailure {
            detail: SourceFailureDetail::RevisionCriterion { .. },
        }
    ));
    assert!(
        report.source_audit.iter().any(|note| matches!(
            note,
            SourceAuditNote::ByteIdentityDiffersFromReference { .. }
        )),
        "revision 失配的提前返回不得吞掉字节身份诊断"
    );
}

/// 相同期望值、相同损坏值、不同分量轴必须是不同证据（tangent 与 up 各
/// 覆盖）：首次 y 分量失配、复算 z 分量失配且数值相同时，证据因字段身份
/// 不同而不等，复算归类必须返回 Untrusted 而非确认 Mismatch。
#[test]
fn same_value_distortion_on_distinct_vector_axes_is_untrusted() {
    use crate::attestation::FirstFailure;
    let evidence_with = |axis: usize, corrupt: fn(&mut crate::spatial::SegmentGeometry)| {
        let mut root = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
        distort(&mut root, |revision| {
            let pose = revision
                .spatial
                .as_mut()
                .expect("spatial")
                .lane_pose
                .as_mut()
                .expect("lane pose");
            assert!(!pose.segments.is_empty(), "fixture must carry segments");
            corrupt(&mut pose.segments[0]);
        });
        let report = concluded(attest(
            &root,
            FULL_SPATIAL,
            expectation_from(root.canonical_origin()),
            SpatialBuildOption::RetainAvailable,
        ));
        assert_eq!(report.verdict, AttestationVerdict::Mismatch);
        let evidence = report.mismatch.expect("evidence");
        let _ = axis;
        evidence
    };
    type Corrupt = fn(&mut crate::spatial::SegmentGeometry);
    let cases: &[(&str, &str, Corrupt, Corrupt)] = &[
        (
            "tangent[1]",
            "tangent[2]",
            (|segment: &mut crate::spatial::SegmentGeometry| {
                segment.tangent[1] = 2.0;
            }) as Corrupt,
            (|segment: &mut crate::spatial::SegmentGeometry| {
                segment.tangent[2] = 2.0;
            }) as Corrupt,
        ),
        (
            "up[1]",
            "up[2]",
            (|segment: &mut crate::spatial::SegmentGeometry| {
                segment.up[1] = 2.0;
            }) as Corrupt,
            (|segment: &mut crate::spatial::SegmentGeometry| {
                segment.up[2] = 2.0;
            }) as Corrupt,
        ),
    ];
    for (field_y, field_z, corrupt_y, corrupt_z) in cases {
        let first = evidence_with(1, *corrupt_y);
        let second = evidence_with(2, *corrupt_z);
        assert_ne!(first, second, "不同分量轴的失配是不同证据");
        assert!(matches!(
            (&first.site, &second.site),
            (
                MismatchSite::TableValue { field: first_field, .. },
                MismatchSite::TableValue { field: second_field, .. }
            ) if *first_field == *field_y && *second_field == *field_z
        ));
        let outcome = attestation::classify_confirmed_failure(
            FirstFailure::RootCompare(first),
            AttemptOutcome::Failed(FirstFailure::RootCompare(second)),
            Vec::new(),
        );
        match outcome {
            AttestationOutcome::Concluded(report) => {
                assert_eq!(report.verdict, AttestationVerdict::Untrusted);
                assert_eq!(report.cause, Some(AttestationCause::Indeterminate));
            }
            other => panic!("expected Untrusted, got {other:?}"),
        }
    }
}

#[derive(Default)]
struct RecordingSink {
    events: Vec<&'static str>,
}

impl AttestationTelemetry for RecordingSink {
    fn emit(&mut self, event: AttestationEvent<'_>) -> Result<(), TelemetryDeliveryError> {
        let label = match event {
            AttestationEvent::Trace(_) => "trace",
            AttestationEvent::Verdict { .. } => "verdict",
            AttestationEvent::Mismatch(_) => "mismatch",
            AttestationEvent::SourceAudit(_) => "source_audit",
            AttestationEvent::Disposition { .. } => "disposition",
        };
        self.events.push(label);
        Ok(())
    }
}

#[test]
fn telemetry_event_sequence_l0_to_l3() {
    let mut root = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
    distort(&mut root, |revision| {
        let column = &mut revision.planning_hints.edge_boundary_weights;
        column[0] = column[0].wrapping_add(1);
    });
    let mut sink = RecordingSink::default();
    let outcome = attest_shared_network_revision(
        &root,
        FULL_SPATIAL,
        expectation_from(root.canonical_origin()),
        FormatLimits::HARD,
        SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, BUILD_LIMITS),
        Some(&mut sink),
    );
    let report = concluded(outcome);
    assert_eq!(report.verdict, AttestationVerdict::Mismatch);
    assert!(sink.events.contains(&"trace"), "L0 进度事件");
    assert!(sink.events.contains(&"verdict"), "L1 结论事件");
    assert!(sink.events.contains(&"mismatch"), "L2 失配事件");
    assert!(sink.events.contains(&"disposition"), "L3 处置事件");
    let verdict_position = sink
        .events
        .iter()
        .position(|e| *e == "verdict")
        .expect("L1");
    assert!(
        sink.events[..verdict_position]
            .iter()
            .all(|e| *e == "trace"),
        "结论事件在 L0 之后发出"
    );
}

struct PanickingSink;

#[derive(Default)]
struct RejectingDispositionSink {
    accepted: u32,
    rejected: u32,
    disposition: Option<Disposition>,
}

impl AttestationTelemetry for RejectingDispositionSink {
    fn emit(&mut self, event: AttestationEvent<'_>) -> Result<(), TelemetryDeliveryError> {
        if let AttestationEvent::Disposition { disposition, .. } = event {
            self.rejected += 1;
            self.disposition = Some(disposition);
            Err(TelemetryDeliveryError)
        } else {
            self.accepted += 1;
            Ok(())
        }
    }
}

#[test]
fn telemetry_delivery_error_is_counted_without_changing_outcome() {
    let root = build(MIN_HEADLESS, SpatialBuildOption::RetainAvailable);
    for cancel in [false, true] {
        let cancelled = AtomicBool::new(cancel);
        let mut sink = RejectingDispositionSink::default();
        let outcome = attest_shared_network_revision(
            &root,
            MIN_HEADLESS,
            expectation_from(root.canonical_origin()),
            FormatLimits::HARD,
            SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, BUILD_LIMITS)
                .with_cancellation(&cancelled),
            Some(&mut sink),
        );
        assert!(sink.accepted > 0);
        assert_eq!(sink.rejected, 1);
        if cancel {
            let AttestationOutcome::OperationFailed(failure) = outcome else {
                panic!("expected cancellation");
            };
            assert_eq!(failure.kind, OperationFailureKind::Cancelled);
            assert_eq!(failure.telemetry_dropped, 1);
            assert_eq!(sink.disposition, Some(Disposition::Retry));
        } else {
            let report = concluded(outcome);
            assert_eq!(report.verdict, AttestationVerdict::Pass);
            assert_eq!(report.cause, None);
            assert_eq!(report.disposition, Disposition::None);
            assert_eq!(report.telemetry_dropped, 1);
            assert_eq!(sink.disposition, Some(Disposition::None));
        }
    }
}

impl AttestationTelemetry for PanickingSink {
    fn emit(&mut self, _event: AttestationEvent<'_>) -> Result<(), TelemetryDeliveryError> {
        panic!("telemetry sink failure");
    }
}

/// 遥测 sink 失败或 panic 不改变已确定的认证结论（设计 §7）。
#[test]
fn telemetry_sink_panic_is_absorbed() {
    let root = build(MIN_HEADLESS, SpatialBuildOption::RetainAvailable);
    let mut sink = PanickingSink;
    let outcome = attest_shared_network_revision(
        &root,
        MIN_HEADLESS,
        expectation_from(root.canonical_origin()),
        FormatLimits::HARD,
        SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, BUILD_LIMITS),
        Some(&mut sink),
    );
    let report = concluded(outcome);
    assert_eq!(report.verdict, AttestationVerdict::Pass);
    assert!(
        report.telemetry_dropped > 0,
        "sink panic 记为遥测丢失并随结果返回，不经故障 sink 报告"
    );
}

/// 保留的来源审计诊断经 SourceAudit 事件进入遥测（L2 补充），覆盖瞬时
/// 恢复与复算未完成两条真实路径；证据不稳定经 emit_telemetry 直测；新事件
/// 投递失败计入 telemetry_dropped 且不改变结论。
#[test]
fn source_audit_notes_reach_telemetry_on_both_paths() {
    // 瞬时恢复：首次来源失败、复算通过 → Pass 仍携带 SourceAudit 事件。
    let root = build(MIN_HEADLESS, SpatialBuildOption::RetainAvailable);
    test_injection::arm_source_preparation_panic_once();
    let mut sink = RecordingSink::default();
    let outcome = attest_shared_network_revision(
        &root,
        MIN_HEADLESS,
        expectation_from(root.canonical_origin()),
        FormatLimits::HARD,
        SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, BUILD_LIMITS),
        Some(&mut sink),
    );
    test_injection::reset();
    let report = concluded(outcome);
    assert_eq!(report.verdict, AttestationVerdict::Pass);
    assert!(
        sink.events.contains(&"source_audit"),
        "Pass 携带来源诊断事件"
    );
    assert!(
        !sink.events.contains(&"mismatch"),
        "来源诊断不冒充 Mismatch 事件"
    );

    // 复算未完成：操作失败路径同样发出保留诊断。
    let root = build(MIN_HEADLESS, SpatialBuildOption::RetainAvailable);
    test_injection::arm_source_preparation_panic_once();
    arm_allocation_failure_injection(1);
    let mut sink = RecordingSink::default();
    let outcome = attest_shared_network_revision(
        &root,
        MIN_HEADLESS,
        expectation_from(root.canonical_origin()),
        FormatLimits::HARD,
        SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, BUILD_LIMITS),
        Some(&mut sink),
    );
    test_injection::reset();
    reset_allocation_failure_injection();
    assert!(matches!(
        outcome,
        AttestationOutcome::OperationFailed(ref failure)
            if failure.kind == OperationFailureKind::RecomputeIncomplete
    ));
    assert!(
        sink.events.contains(&"source_audit"),
        "操作失败路径发出 SourceRecomputeIncomplete 诊断事件"
    );

    // 证据不稳定（操作失败）：emit_telemetry 直测——真实路径无法构造两次
    // 不同的来源失败证据。
    let unstable = AttestationOutcome::OperationFailed(crate::attestation::OperationFailure {
        kind: OperationFailureKind::SourceEvidenceUnstable,
        detail: "unstable".to_string(),
        source_audit: vec![SourceAuditNote::SourceEvidenceUnstable {
            first: Box::new(MismatchEvidence {
                stage: attestation::AttestationStage::SourcePreparation,
                site: MismatchSite::SourceFailure {
                    detail: SourceFailureDetail::Panic("a".to_string()),
                },
            }),
            recompute: Box::new(MismatchEvidence {
                stage: attestation::AttestationStage::SourcePreparation,
                site: MismatchSite::SourceFailure {
                    detail: SourceFailureDetail::Panic("b".to_string()),
                },
            }),
        }],
        telemetry_dropped: 0,
    });
    let mut unstable = unstable;
    let mut sink = RecordingSink::default();
    let dropped = attestation::emit_telemetry(Some(&mut sink), &mut unstable, &[]);
    assert_eq!(dropped, 0);
    assert!(sink.events.contains(&"source_audit"));

    // 新事件投递失败：计入 telemetry_dropped，不改变结论。
    let root = build(MIN_HEADLESS, SpatialBuildOption::RetainAvailable);
    test_injection::arm_source_preparation_panic_once();
    let mut sink = RejectingSourceAuditSink::default();
    let outcome = attest_shared_network_revision(
        &root,
        MIN_HEADLESS,
        expectation_from(root.canonical_origin()),
        FormatLimits::HARD,
        SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, BUILD_LIMITS),
        Some(&mut sink),
    );
    test_injection::reset();
    let report = concluded(outcome);
    assert_eq!(
        report.verdict,
        AttestationVerdict::Pass,
        "投递失败不改变结论"
    );
    assert_eq!(report.telemetry_dropped, 1, "SourceAudit 投递失败计入丢失");
    assert_eq!(sink.rejected, 1);
}

#[derive(Default)]
struct RejectingSourceAuditSink {
    rejected: u32,
}

impl AttestationTelemetry for RejectingSourceAuditSink {
    fn emit(&mut self, event: AttestationEvent<'_>) -> Result<(), TelemetryDeliveryError> {
        match event {
            AttestationEvent::SourceAudit(_) => {
                self.rejected += 1;
                Err(TelemetryDeliveryError)
            }
            _ => Ok(()),
        }
    }
}

/// gates / targets 复合记录失配报告具名字段（rule / passage_local_index），
/// 不再统一为 "value"。
#[test]
fn gate_policy_and_yield_target_report_named_members() {
    let mut root = build(POLICY_FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
    distort(&mut root, |revision| {
        assert!(
            !revision.policy.gates.is_empty(),
            "fixture must carry resolved gate policies"
        );
        revision.policy.gates[0].rule = revision.policy.gates[0].rule.wrapping_add(1);
    });
    let report = concluded(attest(
        &root,
        POLICY_FULL_SPATIAL,
        expectation_from(root.canonical_origin()),
        SpatialBuildOption::RetainAvailable,
    ));
    assert_eq!(report.verdict, AttestationVerdict::Mismatch);
    assert!(matches!(
        report.mismatch.as_ref().expect("evidence").site,
        MismatchSite::TableValue {
            component: RootComponent::Policy,
            table: "gates",
            field: "rule",
            ..
        }
    ));
}

/// 认证无常驻状态：同一修订连续两次认证结论一致（L1 不跨调用保留状态）。
#[test]
fn attestation_is_stateless_across_calls() {
    let root = build(MIN_HEADLESS, SpatialBuildOption::RetainAvailable);
    let first = concluded(attest(
        &root,
        MIN_HEADLESS,
        expectation_from(root.canonical_origin()),
        SpatialBuildOption::RetainAvailable,
    ));
    let second = concluded(attest(
        &root,
        MIN_HEADLESS,
        expectation_from(root.canonical_origin()),
        SpatialBuildOption::RetainAvailable,
    ));
    assert_eq!(first.verdict, second.verdict);
    assert_eq!(first.cause, second.cause);
    assert_eq!(first.mismatch, second.mismatch);
}

#[test]
fn disposition_guidance_differs_by_cause_and_timing() {
    let dispositions = [
        Disposition::None,
        Disposition::DisableRoot,
        Disposition::IsolateSource,
        Disposition::Retry,
    ];
    for disposition in dispositions {
        let pre_use = disposition.pre_use_guidance();
        let in_use = disposition.in_use_guidance();
        assert!(!pre_use.is_empty());
        assert!(!in_use.is_empty());
    }
    // 判因来源的提示不指向修订重建；停用类提示指向停用并重建。
    assert!(
        Disposition::IsolateSource
            .in_use_guidance()
            .contains("不停用"),
        "来源判因在用提示不停用修订"
    );
    assert!(
        Disposition::DisableRoot.in_use_guidance().contains("重建"),
        "停用类处置指向修订重建"
    );
}
