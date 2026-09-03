//! `CompilationOutput` 到 LFCA/LFSM/LFSD 的原子可移植候选发射。
//!
//! 本模块拥有编译器私有 LIR/source-map 语义投影、摘要和跨对象绑定。线格式的结构、
//! 编码和值域预检仍只由 `laneflow-format` 提供；通用 binding 检查与 LFCP 构造不属于 emitter，
//! LFSD 路权差异在发射后交给独立策略检查器核对实际两根，
//! exact bytes 的持久化、认证与发布由宿主负责。

mod api;
mod lfca;
mod lfsd;
mod lfsm;
mod model;
mod relations;
mod wire;

pub use api::{
    PortableDiffBase, PortableEmissionError, PortableEmissionProvenance, PortableObjectCandidate,
    PortablePublicationCandidate,
};
pub(crate) use api::{close_object, close_staged_object, object_key, sha256};
use lfca::build_lfca;
pub use lfsd::check_portable_policy_diff;
use lfsd::{build_lfsd, verify_target_relation_projection};
use lfsm::build_lfsm;
pub use lfsm::check_portable_policy_sources;
use model::*;
use wire::*;

use laneflow_format::{
    ClosedStagedObjectSource, ExpectedSemanticDiffBase, FieldWriteInput, FieldWriteValue,
    FormatError, FormatLimits, ImmutableObjectSource, ObjectSourceError, ObjectWriteInput,
    RegistryCheckedFieldValue, RegistryCheckedObjectView, RegistryCheckedOrdinalVectorView,
    RegistryCheckedRecordVectorView, RegistryCheckedRowView, RowWriteInput, SectionWriteInput,
    StagedObjectError, StagedObjectWriter, TableWriteInput, ValueCheckedObjectView,
    encode_prepared_object, preflight_object_values, prepare_object,
};
use laneflow_static_contract::{
    CANONICAL_ARTIFACT_FORMAT_VERSION, CONSTRAINT_CONTRACT_VERSION, EntityKind, EntityKindMarker,
    ExactByteLength, IDENTITY_ENCODING_VERSION, IDENTITY_REGISTRY_REVISION,
    NETWORK_REVISION_DERIVATION_VERSION, NETWORK_REVISION_DOMAIN_PREFIX, NetworkRevisionId,
    Ordinal, OrdinalKind, PortableObjectKind, STATIC_EXECUTION_CONTRACT_VERSION, Sha256Digest,
    StableId,
};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};

use crate::{CompilationOutput, CompileLimitDimension};

const SOURCE_COLLECTION_DIGEST_VERSION_V1: u16 = 1;
const CHUNKED_EMITTER_VERSION: u16 = 2;
const SOURCE_COLLECTION_DOMAIN_V1: &[u8] = b"laneflow.source-collection.v1\0";
const PORTABLE_COMPILE_OPTIONS_DIGEST_V1: [u8; 32] = [
    0x32, 0x26, 0x82, 0xf4, 0x55, 0xd0, 0x6b, 0x36, 0xe9, 0xe3, 0x71, 0x9f, 0x34, 0x1d, 0xb3, 0x8f,
    0x3e, 0xcd, 0xa6, 0x1d, 0x52, 0xc5, 0x3d, 0x9d, 0x6f, 0xe3, 0xdc, 0xa5, 0x40, 0xee, 0xf4, 0x45,
];

fn stable_id_bytes<K: EntityKindMarker>(id: StableId<K>) -> [u8; 16] {
    id.into_untyped().into_bytes()
}

fn source_relation_role_code(value: crate::SourceRelationRole) -> u8 {
    match value {
        crate::SourceRelationRole::LaneEdgeSuccessor => 1,
        crate::SourceRelationRole::RoadCorridorElement => 2,
        crate::SourceRelationRole::RoadSectionLane => 3,
        crate::SourceRelationRole::AuthoringLaneEdge => 4,
        crate::SourceRelationRole::LaneGroupMember => 5,
        crate::SourceRelationRole::JunctionMovement => 6,
        crate::SourceRelationRole::MovementManeuverPath => 7,
        crate::SourceRelationRole::ManeuverPathEdge => 8,
        crate::SourceRelationRole::JunctionInternalEdge => 9,
        crate::SourceRelationRole::ManeuverPathGate => 10,
        crate::SourceRelationRole::ManeuverPathWaitingZone => 11,
        crate::SourceRelationRole::StopLineManeuverGate => 12,
        crate::SourceRelationRole::ParkingFacilityVirtualEntry => 13,
        crate::SourceRelationRole::ParkingFacilityVirtualExit => 14,
        crate::SourceRelationRole::JunctionConflictZone => 15,
        crate::SourceRelationRole::JunctionParticipantStream => 16,
        crate::SourceRelationRole::SignalControllerGroup => 17,
        crate::SourceRelationRole::SignalControllerPhase => 18,
        crate::SourceRelationRole::SignalPhaseState => 19,
        crate::SourceRelationRole::ManeuverGateSignalGroup => 20,
        crate::SourceRelationRole::ParkingSpaceFacility => 21,
        crate::SourceRelationRole::ParkingSpaceEntry => 22,
        crate::SourceRelationRole::ParkingSpaceExit => 23,
        crate::SourceRelationRole::ParticipantClassExtends => 24,
        crate::SourceRelationRole::AccessRuleTarget => 25,
        crate::SourceRelationRole::AccessRuleParticipantClass => 26,
        crate::SourceRelationRole::VehicleProfileParticipantClass => 27,
        crate::SourceRelationRole::CanonicalFrameLaneEdgeGeometry => 28,
        crate::SourceRelationRole::CanonicalFrameFacilityBandGeometry => 29,
        crate::SourceRelationRole::ParticipantStreamManeuverPath => 30,
        crate::SourceRelationRole::ParticipantStreamConflictPassage => 31,
        crate::SourceRelationRole::CanonicalFrameConflictZoneRegion => 32,
        crate::SourceRelationRole::PolicyEvidence => 33,
        crate::SourceRelationRole::PolicyGapProfile => 34,
        crate::SourceRelationRole::PolicyStreamRule => 35,
        crate::SourceRelationRole::PolicyGateRule => 36,
    }
}

fn geometry_accuracy_profile_code(value: crate::GeometryAccuracyProfile) -> u8 {
    match value {
        crate::GeometryAccuracyProfile::Fine2Cm => 1,
        crate::GeometryAccuracyProfile::Balanced5Cm => 2,
        crate::GeometryAccuracyProfile::Compact10Cm => 3,
    }
}

fn geometry_direction_profile_code(value: crate::GeometryDirectionProfile) -> u8 {
    match value {
        crate::GeometryDirectionProfile::Smooth1Deg => 1,
        crate::GeometryDirectionProfile::Balanced2Deg => 2,
        crate::GeometryDirectionProfile::Compact5Deg => 3,
    }
}

/// 从同一个成功编译结果原子发射 LFCA/LFSM/LFSD 候选。
///
pub fn emit_portable_candidate(
    output: &CompilationOutput,
    provenance: &PortableEmissionProvenance,
    limits: FormatLimits,
    base: PortableDiffBase<'_>,
) -> Result<PortablePublicationCandidate, PortableEmissionError> {
    emit_portable_candidate_with(output, provenance, limits, base, |object, object_limit| {
        Ok(close_object(encode_owned_object(
            object,
            limits,
            object_limit,
        )?))
    })
}

/// 把三份百万级候选直接发射到调用方选择的临时目录，并在返回前关闭 LaneFlow 的全部
/// 写能力。返回候选、checker 与共享静态构建复用同一 file backing。
pub fn emit_portable_candidate_to_staging(
    output: &CompilationOutput,
    provenance: &PortableEmissionProvenance,
    limits: FormatLimits,
    base: PortableDiffBase<'_>,
    staging_directory: &Path,
) -> Result<PortablePublicationCandidate, PortableEmissionError> {
    emit_portable_candidate_with(output, provenance, limits, base, |object, object_limit| {
        close_staged_object(stage_owned_object(
            object,
            limits,
            object_limit,
            staging_directory,
        )?)
    })
}

fn emit_portable_candidate_with(
    output: &CompilationOutput,
    provenance: &PortableEmissionProvenance,
    limits: FormatLimits,
    base: PortableDiffBase<'_>,
    mut emit_object: impl FnMut(
        &OwnedObject,
        Option<u64>,
    ) -> Result<PortableObjectCandidate, PortableEmissionError>,
) -> Result<PortablePublicationCandidate, PortableEmissionError> {
    let compile_limits = output.compile_limits();
    let object_limit = compile_limits.max_portable_object_bytes();
    let bundle_limit = compile_limits.max_portable_bundle_bytes();
    let mut bundle_bytes = 0_u64;
    let source_collection_digest = source_collection_digest(output)?;
    let mut lfca = build_lfca(
        output,
        provenance,
        source_collection_digest,
        NetworkRevisionId::from_digest(Sha256Digest::ZERO),
    )?;
    let preliminary_lfca = emit_object(&lfca, object_limit)?;
    let network_revision = network_revision(preliminary_lfca.bytes(), limits)?;
    drop(preliminary_lfca);
    set_lfca_network_revision(&mut lfca, network_revision)?;
    let canonical_artifact = emit_object(&lfca, object_limit)?;
    add_to_portable_bundle(
        &mut bundle_bytes,
        canonical_artifact.byte_length(),
        bundle_limit,
    )?;
    let canonical_view = preflight_object_values(
        canonical_artifact.bytes(),
        PortableObjectKind::CanonicalArtifact,
        limits,
    )?
    .registry_view();
    verify_target_relation_projection(output, canonical_view)?;
    drop(lfca);

    let lfsm = build_lfsm(
        output,
        provenance,
        source_collection_digest,
        network_revision,
        &canonical_artifact,
    )?;
    let source_map = emit_object(&lfsm, object_limit)?;
    add_to_portable_bundle(&mut bundle_bytes, source_map.byte_length(), bundle_limit)?;
    preflight_object_values(source_map.bytes(), PortableObjectKind::SourceMap, limits)?;
    drop(lfsm);
    check_portable_policy_sources(
        canonical_artifact.bytes(),
        output.source_map_input(),
        source_map.bytes(),
        limits,
        compile_limits,
    )?;

    let (lfsd, expected_semantic_diff_base) =
        build_lfsd(output, base, network_revision, &canonical_artifact, limits)?;
    let semantic_diff = emit_object(&lfsd, object_limit)?;
    add_to_portable_bundle(&mut bundle_bytes, semantic_diff.byte_length(), bundle_limit)?;
    preflight_object_values(
        semantic_diff.bytes(),
        PortableObjectKind::SemanticDiff,
        limits,
    )?;
    drop(lfsd);
    check_portable_policy_diff(
        base,
        canonical_artifact.bytes(),
        semantic_diff.bytes(),
        limits,
        compile_limits,
    )?;

    Ok(PortablePublicationCandidate {
        canonical_artifact,
        source_map,
        semantic_diff,
        network_revision,
        compiler_build_id: provenance.compiler_build_id.clone(),
        source_collection_digest_version: SOURCE_COLLECTION_DIGEST_VERSION_V1,
        source_collection_digest,
        expected_semantic_diff_base,
    })
}

fn add_to_portable_bundle(
    total: &mut u64,
    byte_length: ExactByteLength,
    limit: Option<u64>,
) -> Result<(), PortableEmissionError> {
    let actual = total
        .checked_add(byte_length.get())
        .ok_or(PortableEmissionError::ArithmeticOverflow)?;
    if let Some(limit) = limit
        && actual > limit
    {
        return Err(PortableEmissionError::CompileLimitExceeded {
            dimension: CompileLimitDimension::PortableBundleBytes,
            actual,
            limit,
        });
    }
    *total = actual;
    Ok(())
}

fn source_collection_digest(output: &CompilationOutput) -> Result<[u8; 32], PortableEmissionError> {
    source_collection_digest_from_map(output.source_map_input())
}

fn source_collection_digest_from_map(
    source: &crate::ValidatedSourceMapInput,
) -> Result<[u8; 32], PortableEmissionError> {
    let modules = source.source_modules();
    let module_count =
        u32::try_from(modules.len()).map_err(|_| PortableEmissionError::ArithmeticOverflow)?;
    let mut hasher = Sha256::new();
    hasher.update(SOURCE_COLLECTION_DOMAIN_V1);
    hasher.update(module_count.to_le_bytes());
    for module in modules {
        let namespace = module.authoring_namespace_id().as_bytes();
        let namespace_length = u32::try_from(namespace.len())
            .map_err(|_| PortableEmissionError::ArithmeticOverflow)?;
        hasher.update(namespace_length.to_le_bytes());
        hasher.update(namespace);
        hasher.update(module.source_document_set_digest_version().to_le_bytes());
        hasher.update(module.source_document_set_digest());
    }
    Ok(hasher.finalize().into())
}

fn network_revision(
    bytes: &[u8],
    limits: FormatLimits,
) -> Result<NetworkRevisionId, PortableEmissionError> {
    let view = preflight_object_values(bytes, PortableObjectKind::CanonicalArtifact, limits)?;
    network_revision_from_checked(view)
}

fn network_revision_from_checked(
    view: ValueCheckedObjectView<'_>,
) -> Result<NetworkRevisionId, PortableEmissionError> {
    let view = view.registry_view();
    let mut hasher = Sha256::new();
    hasher.update(NETWORK_REVISION_DOMAIN_PREFIX);
    for ordinal in 0..6 {
        let section = view
            .section(ordinal)
            .ok_or(PortableEmissionError::InternalBindingMismatch)?;
        let section_kind =
            u16::try_from(ordinal + 1).map_err(|_| PortableEmissionError::ArithmeticOverflow)?;
        hasher.update(section_kind.to_le_bytes());
        hasher.update(
            PortableObjectKind::CanonicalArtifact
                .section_format_version()
                .to_le_bytes(),
        );
        hasher.update(
            u64::try_from(section.bytes().len())
                .expect("supported targets have at most 64-bit usize")
                .to_le_bytes(),
        );
        hasher.update(section.bytes());
    }
    Ok(NetworkRevisionId::from_digest(Sha256Digest::from_bytes(
        hasher.finalize().into(),
    )))
}

fn set_lfca_network_revision(
    object: &mut OwnedObject,
    revision: NetworkRevisionId,
) -> Result<(), PortableEmissionError> {
    let value = object
        .sections
        .get_mut(7)
        .and_then(|section| section.tables.get_mut(0))
        .and_then(|table| table.rows.get_mut(0))
        .and_then(|row| row.fields.get_mut(0))
        .ok_or(PortableEmissionError::InternalBindingMismatch)?;
    value.value = OwnedValue::Sha256(revision.into_digest().into_bytes());
    Ok(())
}
