//! 共享路网修订认证的编辑来源分支（#529；完整性设计 §2.2、§4.2）。宿主重编译由
//! 上层测试的组合路径提供：健康修订、`RoadEditingState` 失真、派生表失真、内容失败与
//! 宿主参数不一致。

use super::*;
use crate::attest_editable_shared_network_revision;
use laneflow_static_network::{
    AttestationCause, AttestationOutcome, AttestationReport, AttestationVerdict, MismatchSite,
    OriginField, RecompileFailure, RootComponent, SourceFailureDetail,
};

const PROVENANCE: &str = "runtime-editable-attestation";

fn build_options() -> SharedNetworkBuildOptions<'static> {
    SharedNetworkBuildOptions::new(
        SpatialBuildOption::RetainAvailable,
        SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
    )
}

/// 宿主重编译闭包：任一模块无法准入或编译时报告内容失败。
fn host_recompile(
    provenance: &'static str,
) -> impl FnMut(&RoadEditingState) -> Result<Vec<u8>, RecompileFailure> {
    move |state| {
        let limits = CompileLimits::p100_initial_v2();
        let content = |error: &dyn std::fmt::Debug| RecompileFailure::Content(format!("{error:?}"));
        let mut unit = CompilationUnitBuilder::new(limits);
        for module in state.modules() {
            let input = lfre::RoadEditingModuleInput::try_new(
                module.source_document_key(),
                module.source_bytes(),
                None,
            )
            .map_err(|error| content(&error))?;
            unit.add_road_editing_module(input)
                .map_err(|error| content(&error))?;
        }
        let output = Compiler::new()
            .compile(unit.build().map_err(|error| content(&error))?)
            .map_err(|bundle| content(&bundle.diagnostics()))?;
        let provenance =
            PortableEmissionProvenance::try_new(provenance).map_err(|error| content(&error))?;
        let candidate = emit_portable_candidate(
            &output,
            &provenance,
            FormatLimits::HARD,
            PortableDiffBase::Genesis,
        )
        .map_err(|error| content(&error))?;
        Ok(candidate.canonical_artifact().bytes().to_vec())
    }
}

fn attest(
    root: &SharedNetworkRevision,
    state: &RoadEditingState,
    provenance: &'static str,
) -> AttestationReport {
    match attest_editable_shared_network_revision(
        root,
        state,
        host_recompile(provenance),
        FormatLimits::HARD,
        build_options(),
        None,
    ) {
        AttestationOutcome::Concluded(report) => report,
        AttestationOutcome::OperationFailed(failure) => {
            panic!("expected concluded outcome, got operation failure: {failure:?}")
        }
    }
}

#[test]
fn healthy_editable_root_passes() {
    let state = state(false);
    let compiled = recompile(&state, PROVENANCE, None);
    let report = attest(&compiled.revision, &state, PROVENANCE);
    assert_eq!(report.verdict, AttestationVerdict::Pass);
    assert!(report.source_audit.is_empty());
}

/// 宿主持有的状态与构建修订所用状态不同但仍可编译：`NetworkRevisionId` 失配，判因
/// `无法判因`（完整性设计 §2.2 编辑来源期望值）。
#[test]
fn road_editing_state_distortion_is_indeterminate() {
    let built_from = state(false);
    let compiled = recompile(&built_from, PROVENANCE, None);
    let distorted = state(true);
    assert_ne!(distorted.digest(), built_from.digest());
    let report = attest(&compiled.revision, &distorted, PROVENANCE);
    assert_eq!(report.verdict, AttestationVerdict::Mismatch);
    assert_eq!(report.cause, Some(AttestationCause::Indeterminate));
    assert!(matches!(
        report.mismatch.expect("evidence").site,
        MismatchSite::OriginField {
            field: OriginField::NetworkRevision,
            ..
        }
    ));
}

#[test]
fn derived_table_distortion_is_shared_static_root() {
    let state = state(false);
    let mut compiled = recompile(&state, PROVENANCE, None);
    laneflow_static_network::attestation_fixtures::perturb_edge_boundary_weight(
        Arc::get_mut(&mut compiled.revision).expect("unique test revision"),
        0,
    );
    let report = attest(&compiled.revision, &state, PROVENANCE);
    assert_eq!(report.verdict, AttestationVerdict::Mismatch);
    assert_eq!(report.cause, Some(AttestationCause::SharedStaticRoot));
    assert!(matches!(
        report.mismatch.expect("evidence").site,
        MismatchSite::TableValue {
            component: RootComponent::PlanningHints,
            ..
        }
    ));
}

/// 状态无法编译：来源准备阶段失败，判因 `来源`，修订不被证伪。
#[test]
fn uncompilable_state_is_source_cause() {
    let state = state(false);
    let compiled = recompile(&state, PROVENANCE, None);
    let broken = RoadEditingState::try_new(vec![RoadEditingStateModule::new(
        DOCUMENT_KEY.to_owned(),
        vec![0; 16],
    )])
    .expect("structurally valid state");
    let report = attest(&compiled.revision, &broken, PROVENANCE);
    assert_eq!(report.verdict, AttestationVerdict::Mismatch);
    assert_eq!(report.cause, Some(AttestationCause::Source));
    assert!(matches!(
        report.mismatch.expect("evidence").site,
        MismatchSite::SourceFailure {
            detail: SourceFailureDetail::Recompile(_),
        }
    ));
}

/// 宿主以不同参数重编译（此处为发射 provenance）：字节身份失配，判因 `无法判因`；
/// 这是宿主调用错误，不是修订失真（`committed-road-editing-state.md` §4）。
#[test]
fn host_parameter_mismatch_is_indeterminate_byte_identity() {
    let state = state(false);
    let compiled = recompile(&state, PROVENANCE, None);
    let report = attest(
        &compiled.revision,
        &state,
        "runtime-editable-attestation-other",
    );
    assert_eq!(report.verdict, AttestationVerdict::Mismatch);
    assert_eq!(report.cause, Some(AttestationCause::Indeterminate));
    assert!(matches!(
        report.mismatch.expect("evidence").site,
        MismatchSite::OriginField {
            field: OriginField::CanonicalArtifactDigest,
            ..
        }
    ));
}
