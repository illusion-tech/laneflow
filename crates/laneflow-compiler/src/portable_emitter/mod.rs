//! `CompilationOutput` 到 LFCA/LFSM/LFSD 的原子可移植候选发射。
//!
//! 本模块拥有编译器私有 LIR/source-map 语义投影、摘要和跨对象绑定。线格式的结构、
//! 编码和值域预检仍只由 `laneflow-format` 提供；文件系统安装由独立 `portable_store`
//! 模块负责；后发射检查、LFCP 与 manifest 提交不属于 emitter。

mod api;
mod lfca;
mod lfsd;
mod lfsm;
mod model;
mod relations;
mod wire;

pub use api::{
    PortableDiffBase, PortableEmissionError, PortableEmissionProvenanceV1, PortableObjectCandidate,
    PortablePublicationCandidate,
};
pub(crate) use api::{close_object, object_key, sha256};
use lfca::build_lfca;
use lfsd::{build_lfsd, verify_target_relation_projection};
use lfsm::build_lfsm;
use model::*;
use wire::*;

use laneflow_format::{
    ExpectedSemanticDiffBaseV1, FieldWriteInputV1, FieldWriteValueV1, FormatError, FormatLimits,
    ObjectWriteInputV1, RegistryCheckedFieldValue, RegistryCheckedObjectView,
    RegistryCheckedOrdinalVectorView, RegistryCheckedRecordVectorView, RegistryCheckedRowView,
    RowWriteInputV1, SectionWriteInputV1, TableWriteInputV1, ValueCheckedObjectView,
    encode_prepared_object_v1, preflight_object_values_v1, prepare_object_v1,
};
use laneflow_static_contract::{
    CANONICAL_ARTIFACT_FORMAT_VERSION, EntityKind, EntityKindMarker, ExactByteLength,
    IDENTITY_ENCODING_VERSION, IDENTITY_REGISTRY_REVISION, NETWORK_REVISION_DERIVATION_VERSION,
    NETWORK_REVISION_DOMAIN_PREFIX, NetworkRevisionId, Ordinal, OrdinalKind, PortableObjectKind,
    SECTION_FORMAT_VERSION_V1, Sha256Digest, StableId,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

use crate::CompilationOutput;

const SOURCE_COLLECTION_DIGEST_VERSION_V1: u16 = 1;
const EMITTER_VERSION_V1: u16 = 1;
const CONSTRAINT_CONTRACT_VERSION_V1: u16 = 1;
const STATIC_EXECUTION_CONTRACT_VERSION_V1: u16 = 1;
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
        crate::SourceRelationRole::StaticRouteEdge => 13,
        crate::SourceRelationRole::StaticRouteManeuverOccurrence => 14,
        crate::SourceRelationRole::StaticRouteGateOccurrence => 15,
        crate::SourceRelationRole::StaticRouteWaitingZoneOccurrence => 16,
        crate::SourceRelationRole::SignalControllerGroup => 17,
        crate::SourceRelationRole::SignalControllerPhase => 18,
        crate::SourceRelationRole::SignalPhaseState => 19,
        crate::SourceRelationRole::ManeuverGateSignalGroup => 20,
        crate::SourceRelationRole::ParkingSpaceArea => 21,
        crate::SourceRelationRole::ParkingSpaceEntry => 22,
        crate::SourceRelationRole::ParkingSpaceExit => 23,
        crate::SourceRelationRole::ParticipantClassExtends => 24,
        crate::SourceRelationRole::AccessRuleTarget => 25,
        crate::SourceRelationRole::AccessRuleParticipantClass => 26,
        crate::SourceRelationRole::VehicleProfileParticipantClass => 27,
        crate::SourceRelationRole::CanonicalFrameLaneEdgeGeometry => 28,
        crate::SourceRelationRole::CanonicalFrameFacilityBandGeometry => 29,
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
    provenance: &PortableEmissionProvenanceV1,
    limits: FormatLimits,
    base: PortableDiffBase<'_>,
) -> Result<PortablePublicationCandidate, PortableEmissionError> {
    let source_collection_digest = source_collection_digest(output)?;
    let mut lfca = build_lfca(
        output,
        provenance,
        source_collection_digest,
        NetworkRevisionId::from_digest(Sha256Digest::ZERO),
    );
    let preliminary_lfca = encode_owned_object(&lfca, limits, 0)?;
    let network_revision = network_revision(&preliminary_lfca, limits)?;
    drop(preliminary_lfca);
    set_lfca_network_revision(&mut lfca, network_revision)?;
    let canonical_artifact = close_object(encode_owned_object(&lfca, limits, 0)?);
    let canonical_view = preflight_object_values_v1(
        canonical_artifact.bytes(),
        PortableObjectKind::CanonicalArtifact,
        limits,
    )?
    .registry_view();
    verify_target_relation_projection(output, canonical_view)?;

    let lfsm = build_lfsm(
        output,
        provenance,
        source_collection_digest,
        network_revision,
        &canonical_artifact,
    )?;
    let source_map = close_object(encode_owned_object(
        &lfsm,
        limits,
        canonical_artifact.byte_length().get(),
    )?);
    preflight_object_values_v1(source_map.bytes(), PortableObjectKind::SourceMap, limits)?;

    let (lfsd, expected_semantic_diff_base) =
        build_lfsd(output, base, network_revision, &canonical_artifact, limits)?;
    let staged_before_diff = canonical_artifact
        .byte_length()
        .get()
        .checked_add(source_map.byte_length().get())
        .ok_or(PortableEmissionError::ArithmeticOverflow)?;
    let semantic_diff = close_object(encode_owned_object(&lfsd, limits, staged_before_diff)?);
    preflight_object_values_v1(
        semantic_diff.bytes(),
        PortableObjectKind::SemanticDiff,
        limits,
    )?;

    let total = canonical_artifact
        .byte_length()
        .get()
        .checked_add(source_map.byte_length().get())
        .and_then(|value| value.checked_add(semantic_diff.byte_length().get()))
        .ok_or(PortableEmissionError::ArithmeticOverflow)?;
    let staging_limit = limits.max_candidate_staging_bytes();
    if total > staging_limit {
        return Err(PortableEmissionError::CandidateStagingLimitExceeded {
            actual: total,
            limit: staging_limit,
        });
    }

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

fn source_collection_digest(output: &CompilationOutput) -> Result<[u8; 32], PortableEmissionError> {
    let modules: Vec<_> = output.source_map_input().source_modules().collect();
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
    let view = preflight_object_values_v1(bytes, PortableObjectKind::CanonicalArtifact, limits)?;
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
        hasher.update(SECTION_FORMAT_VERSION_V1.to_le_bytes());
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
