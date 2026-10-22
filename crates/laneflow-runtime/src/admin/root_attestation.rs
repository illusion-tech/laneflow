//! 发布来源的共享路网修订认证门面（#959 切片 A；完整性设计 §4.2）。
//!
//! 从宿主持有的 [`PublishedLfcaReference`] 组装修订来源期望值并调用
//! `laneflow-static-network` 的认证核心；本 crate 不解析 asset key、不做
//! 文件 I/O。宿主在安装、恢复、Spatial 绑定与切换之前及在用修订审计时调用；
//! 编辑来源（committed `RoadEditingState`）分支随 #529 落地。

use laneflow_format::{BoundedReReadableObjectSource, FormatLimits};
use laneflow_static_network::{
    AttestationOutcome, AttestationTelemetry, RootAttestationExpectation,
    SharedNetworkBuildOptions, SharedNetworkRevision, attest_shared_network_revision,
};

use crate::PublishedLfcaReference;

/// 认证发布来源的共享路网修订（术语表：shared network revision authentication）。
///
/// 期望值组装自 `PublishedLfcaReference`（revision 参与同修订判据；LFCA
/// digest / exact length 只作来源审计诊断）与运行中二进制常量，任何值都
/// 不取自被认证修订自身的 `origin`。`format_limits` 与 `build_options`
/// 原样传给 reader / builder（宿主加载 / 构建此修订时使用的同一组策略值）。
///
/// `lfca` 是宿主经资产认证取得、借给认证入口的字节来源：连续切片、
/// staged 对象或映射视图皆可（`BoundedReReadableObjectSource` 的既有实现），
/// 认证在受检读取、来源复核与复算中重复读取同一份来源，不复制制品；
/// 来源视图的建立（含 staged 来源首次映射）由宿主在调用前完成。
///
/// 调用时机与处置按完整性设计 §3 / §7：投入使用之前任何非 `Pass` 都不
/// 投入使用；在用修订审计判因 `来源` 不停用修订，判因 `共享路网修订` /
/// `无法判因` 停用并重建；操作失败不判因、不触发停用。详细失败语义见
/// `laneflow-static-network` 认证入口文档。
///
/// # Panics
///
/// 认证过程按阶段以 `catch_unwind` 包住，panic 不向宿主传播（设计 §2.4）；
/// `panic=abort` 构建下按进程终止语义处理。
pub fn attest_published_shared_network_revision<S>(
    root: &SharedNetworkRevision,
    reference: &PublishedLfcaReference,
    lfca: S,
    format_limits: FormatLimits,
    build_options: SharedNetworkBuildOptions<'_>,
    telemetry: Option<&mut dyn AttestationTelemetry>,
) -> AttestationOutcome
where
    S: BoundedReReadableObjectSource,
{
    let expectation = RootAttestationExpectation::new(
        reference.network_revision(),
        reference.canonical_artifact_digest(),
        reference.canonical_artifact_byte_length(),
    );
    attest_shared_network_revision(
        root,
        lfca,
        expectation,
        format_limits,
        build_options,
        telemetry,
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use laneflow_format::{FormatLimits, check_canonical_network_input};
    use laneflow_static_network::{
        AttestationCause, AttestationOutcome, AttestationVerdict, CanonicalNetworkOrigin,
        SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision,
        SourceAuditNote, SpatialBuildOption, build_shared_network_revision,
    };

    use super::attest_published_shared_network_revision;
    use crate::PublishedLfcaReference;

    const FULL_SPATIAL: &[u8] = include_bytes!(
        "../../../laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfca"
    );
    const PROVENANCE_BASE: &[u8] = include_bytes!(
        "../../../laneflow-compiler/tests/fixtures/portable/lfca-variants/provenance-base.lfca"
    );
    const PROVENANCE_BUILD: &[u8] = include_bytes!(
        "../../../laneflow-compiler/tests/fixtures/portable/lfca-variants/provenance-build.lfca"
    );

    const BUILD_LIMITS: SharedNetworkBuildLimits =
        SharedNetworkBuildLimits::new(64 * 1024 * 1024, 16 * 1024 * 1024);

    fn build(bytes: &[u8]) -> Arc<SharedNetworkRevision> {
        let input = check_canonical_network_input(bytes, FormatLimits::HARD)
            .expect("checked canonical network input");
        build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, BUILD_LIMITS),
        )
        .expect("shared network revision")
    }

    fn reference_for(origin: &CanonicalNetworkOrigin) -> PublishedLfcaReference {
        PublishedLfcaReference::new(
            "host-asset-key",
            origin.canonical_artifact_digest(),
            origin.canonical_artifact_byte_length(),
            origin.network_revision(),
        )
        .expect("valid reference")
    }

    fn attest(
        root: &Arc<SharedNetworkRevision>,
        reference: &PublishedLfcaReference,
        bytes: &[u8],
    ) -> AttestationOutcome {
        attest_published_shared_network_revision(
            root,
            reference,
            bytes,
            FormatLimits::HARD,
            SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, BUILD_LIMITS),
            None,
        )
    }

    #[test]
    fn facade_healthy_published_root_passes() {
        let root = build(FULL_SPATIAL);
        let reference = reference_for(root.canonical_origin());
        match attest(&root, &reference, FULL_SPATIAL) {
            AttestationOutcome::Concluded(report) => {
                assert_eq!(report.verdict, AttestationVerdict::Pass);
                assert!(report.source_audit.is_empty());
            }
            other => panic!("expected concluded outcome, got {other:?}"),
        }
    }

    /// 在用修订审计：修订（base 构建）与同修订另一份引用（build）配对 → Pass
    /// 并记来源审计诊断（Issue #959 验收）。
    #[test]
    fn facade_republished_pairing_passes_with_audit() {
        let base_root = build(PROVENANCE_BASE);
        let build_root = build(PROVENANCE_BUILD);
        let build_reference = reference_for(build_root.canonical_origin());
        match attest(&base_root, &build_reference, PROVENANCE_BASE) {
            AttestationOutcome::Concluded(report) => {
                assert_eq!(report.verdict, AttestationVerdict::Pass);
                assert!(report.source_audit.iter().any(|note| matches!(
                    note,
                    SourceAuditNote::ByteIdentityDiffersFromReference { .. }
                )));
            }
            other => panic!("expected concluded outcome, got {other:?}"),
        }
    }

    /// 引用 revision 损坏：同修订判据失配判给来源而非修订。
    #[test]
    fn facade_reference_revision_corruption_is_source_cause() {
        let root = build(FULL_SPATIAL);
        let healthy = *root.canonical_origin();
        let mut flipped = healthy.network_revision().as_digest().as_bytes().to_owned();
        flipped[0] ^= 0xFF;
        let corrupted = PublishedLfcaReference::new(
            "host-asset-key",
            healthy.canonical_artifact_digest(),
            healthy.canonical_artifact_byte_length(),
            laneflow_static_contract::NetworkRevisionId::from_digest(
                laneflow_static_contract::Sha256Digest::from_bytes(flipped),
            ),
        )
        .expect("valid reference");
        match attest(&root, &corrupted, FULL_SPATIAL) {
            AttestationOutcome::Concluded(report) => {
                assert_eq!(report.verdict, AttestationVerdict::Mismatch);
                assert_eq!(report.cause, Some(AttestationCause::Source));
            }
            other => panic!("expected concluded outcome, got {other:?}"),
        }
    }
}
