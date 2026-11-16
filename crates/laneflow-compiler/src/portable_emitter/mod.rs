//! `CompilationOutput` 到 LFCA/LFSM/LFSD 的原子可移植候选发射。
//!
//! 本模块拥有编译器私有 LIR/source-map 语义投影、摘要和跨对象绑定。线格式的结构、
//! 编码和值域预检仍只由 `laneflow-format` 提供；通用 binding 检查与 LFCP 构造不属于 emitter，
//! LFSD 路权差异在发射后交给独立策略检查器核对实际两份修订，
//! exact bytes 的持久化、认证与发布由宿主负责。

mod api;
mod budget;
mod lfca;
mod lfsd;
mod lfsm;
mod model;
mod relations;
mod tree;
#[cfg(test)]
mod wire;

use api::{OBJECT_KEY_BYTES, close_shared_object};
pub use api::{
    PortableDiffBase, PortableEmissionError, PortableEmissionProvenance,
    PortableEmissionResourceReport, PortableObjectCandidate, PortablePublicationCandidate,
};
pub(crate) use api::{close_object, close_staged_object, object_key, sha256};
use lfca::{build_lfca, set_lfca_tree_revision};
pub use lfsd::check_portable_policy_diff;
use lfsd::{build_lfsd, check_policy_diff, verify_target_relation_projection};
pub use lfsm::check_portable_policy_sources;
use lfsm::{build_lfsm, check_policy_sources};
use model::*;
#[cfg(test)]
use tree::encode_owned_object;
use tree::{ObjectTree, arc_bytes_layout, share_bytes};
#[cfg(test)]
use wire::*;

use laneflow_format::{
    ClosedStagedObjectSource, ExpectedSemanticDiffBase, FormatError, FormatLimits,
    ImmutableObjectSource, ObjectSourceError, RegistryCheckedFieldValue, RegistryCheckedObjectView,
    RegistryCheckedOrdinalVectorView, RegistryCheckedRecordVectorView, RegistryCheckedRowView,
    StagedObjectError, StagedObjectWriter, ValueCheckedObjectView, encode_prepared_object,
    preflight_object_values,
};
use laneflow_static_contract::{
    CANONICAL_ARTIFACT_FORMAT_VERSION, CONSTRAINT_CONTRACT_VERSION, EntityKind, EntityKindMarker,
    ExactByteLength, IDENTITY_ENCODING_VERSION, IDENTITY_REGISTRY_REVISION,
    NETWORK_REVISION_DERIVATION_VERSION, NETWORK_REVISION_DOMAIN_PREFIX, NetworkRevisionId,
    Ordinal, PortableObjectKind, STATIC_EXECUTION_CONTRACT_VERSION, Sha256Digest, StableId,
};
use sha2::{Digest, Sha256};
use std::{alloc::Layout, path::Path};

use crate::ledger::{Grant, Ledger, LedgerStage, Plan};
use crate::{CompilationOutput, CompileLimitDimension};
use budget::Accounting;

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
///
/// # Errors
///
/// 来源摘要派生、对象构建/编码、网络修订派生、格式预检、语义差异基线校验
/// （`base` 非 LFCA 或其语义契约/引用/稳定身份与目标不兼容）、发射后策略闭合
/// 复核（`PolicySourceMismatch` / `PolicyDiffMismatch`）、宿主存续预算或格式对象预算超限
/// 或分配失败时返回相应 [`PortableEmissionError`]；失败不返回部分候选。
pub fn emit_portable_candidate(
    output: &CompilationOutput,
    provenance: &PortableEmissionProvenance,
    limits: FormatLimits,
    base: PortableDiffBase<'_>,
) -> Result<PortablePublicationCandidate, PortableEmissionError> {
    emit_portable_candidate_with(
        output,
        provenance,
        limits,
        base,
        |tree, accounting, points| emit_in_memory(tree, accounting, points, limits),
    )
}

/// 把三份百万级候选直接发射到调用方选择的临时目录，并在返回前关闭 LaneFlow 的全部
/// 写能力。返回候选、checker 与共享静态构建复用同一 file backing。
///
/// # Errors
///
/// 来源摘要派生、对象构建/编码、网络修订派生、格式预检、语义差异基线校验、
/// 发射后策略闭合复核（`PolicySourceMismatch` / `PolicyDiffMismatch`）、
/// 宿主存续预算或格式对象预算超限、分配失败、临时目录 I/O 失败，或封存/首次只读映射时
/// backing 完整性漂移（`StagedBackingChanged` / `ObjectSource`）时返回相应
/// [`PortableEmissionError`]；失败不返回部分候选。
pub fn emit_portable_candidate_to_staging(
    output: &CompilationOutput,
    provenance: &PortableEmissionProvenance,
    limits: FormatLimits,
    base: PortableDiffBase<'_>,
    staging_directory: &Path,
) -> Result<PortablePublicationCandidate, PortableEmissionError> {
    emit_portable_candidate_with(
        output,
        provenance,
        limits,
        base,
        |tree, accounting, points| {
            emit_to_staging(tree, accounting, points, limits, staging_directory)
        },
    )
}

/// 已发射的对象候选，以及仍由发射账本计量、在返回点转交宿主的存储授予。
struct Emitted {
    candidate: PortableObjectCandidate,
    grants: [Option<Grant>; 2],
}

impl Emitted {
    fn bytes(&self) -> &[u8] {
        self.candidate.bytes()
    }

    /// 返回点：候选随返回值离开账本，授予记为转交宿主。
    fn into_host(self) -> PortableObjectCandidate {
        for grant in self.grants.into_iter().flatten() {
            grant.transfer_to_host();
        }
        self.candidate
    }
}

fn object_key_layout() -> Layout {
    Layout::array::<u8>(OBJECT_KEY_BYTES).expect("object key length is a small constant")
}

/// 编码进授予的缓冲，复制为引用计数共享的字节后释放缓冲；计划覆盖两者共存。
fn emit_in_memory(
    tree: &ObjectTree,
    accounting: Accounting<'_>,
    points: (&'static str, &'static str),
    limits: FormatLimits,
) -> Result<Emitted, PortableEmissionError> {
    tree.encode(
        accounting,
        points,
        limits,
        |length| {
            let length = usize::try_from(length).unwrap_or(usize::MAX);
            Plan::ZERO
                .array::<u8>(length)
                .layout(arc_bytes_layout(length))
                .layout(Some(object_key_layout()))
        },
        |prepared, pass| {
            let length = usize::try_from(prepared.byte_len())
                .map_err(|_| PortableEmissionError::ArithmeticOverflow)?;
            let mut buffer = pass.vec::<u8>(length)?;
            buffer.resize(length, 0)?;
            encode_prepared_object(prepared, &mut buffer)?;
            let (shared, bytes_grant) = share_bytes(pass, &buffer)?;
            drop(buffer);
            let key_grant = pass.grant_layout(object_key_layout())?;
            let output = bytes_grant.bytes() + key_grant.bytes();
            Ok((
                Emitted {
                    candidate: close_shared_object(shared),
                    grants: [Some(bytes_grant), Some(key_grant)],
                },
                output,
            ))
        },
    )
}

/// 经 staged writer 写入暂存目录并关闭；暂存目录中的对象字节与只读映射不计入受控存续。
fn emit_to_staging(
    tree: &ObjectTree,
    accounting: Accounting<'_>,
    points: (&'static str, &'static str),
    limits: FormatLimits,
    directory: &Path,
) -> Result<Emitted, PortableEmissionError> {
    tree.encode(
        accounting,
        points,
        limits,
        |_| Plan::ZERO.layout(Some(object_key_layout())),
        |prepared, pass| {
            let source = StagedObjectWriter::create_in(directory, prepared)?.finish()?;
            let key_grant = pass.grant_layout(object_key_layout())?;
            let output = key_grant.bytes();
            Ok((
                Emitted {
                    candidate: close_staged_object(source)?,
                    grants: [Some(key_grant), None],
                },
                output,
            ))
        },
    )
}

fn emit_portable_candidate_with(
    output: &CompilationOutput,
    provenance: &PortableEmissionProvenance,
    limits: FormatLimits,
    base: PortableDiffBase<'_>,
    mut emit_object: impl FnMut(
        &ObjectTree,
        Accounting<'_>,
        (&'static str, &'static str),
    ) -> Result<Emitted, PortableEmissionError>,
) -> Result<PortablePublicationCandidate, PortableEmissionError> {
    // 发射账本：对象树、编码、关系投影自校验、策略来源检查与差异检查共用这一份账本。
    let ledger = Ledger::new(output.compile_limits());
    let single = Accounting {
        ledger: &ledger,
        stage: LedgerStage::SingleEmission,
    };
    let diff = Accounting {
        ledger: &ledger,
        stage: LedgerStage::DiffEmission,
    };
    let source_collection_digest = source_collection_digest(output)?;
    let mut lfca = build_lfca(
        output,
        provenance,
        source_collection_digest,
        NetworkRevisionId::from_digest(Sha256Digest::ZERO),
        single,
    )?;
    let preliminary_lfca = emit_object(&lfca, single, ("lfca-lowering", "lfca-encode"))?;
    let network_revision = network_revision(preliminary_lfca.bytes(), limits)?;
    drop(preliminary_lfca);
    set_lfca_tree_revision(&mut lfca, network_revision)?;
    let canonical_artifact = emit_object(&lfca, single, ("lfca-lowering", "lfca-encode"))?;
    drop(lfca);
    let canonical_view = preflight_object_values(
        canonical_artifact.bytes(),
        PortableObjectKind::CanonicalArtifact,
        limits,
    )?
    .registry_view();
    verify_target_relation_projection(output, canonical_view, single)?;

    let lfsm = build_lfsm(
        output,
        provenance,
        source_collection_digest,
        network_revision,
        &canonical_artifact.candidate,
        single,
    )?;
    let source_map = emit_object(&lfsm, single, ("lfsm-lowering", "lfsm-encode"))?;
    drop(lfsm);
    preflight_object_values(source_map.bytes(), PortableObjectKind::SourceMap, limits)?;
    check_policy_sources(
        single,
        canonical_artifact.bytes(),
        output.source_map_input(),
        source_map.bytes(),
        limits,
    )?;

    let (lfsd, expected_semantic_diff_base) = build_lfsd(
        output,
        base,
        network_revision,
        &canonical_artifact.candidate,
        limits,
        diff,
    )?;
    let semantic_diff = emit_object(&lfsd, diff, ("lfsd-lowering", "lfsd-encode"))?;
    drop(lfsd);
    preflight_object_values(
        semantic_diff.bytes(),
        PortableObjectKind::SemanticDiff,
        limits,
    )?;
    check_policy_diff(
        diff,
        base,
        canonical_artifact.bytes(),
        semantic_diff.bytes(),
        limits,
    )?;

    // 候选携带的编译器构建标识副本同样在返回点转交宿主。
    let metadata = single.admit(
        "candidate-metadata",
        Plan::ZERO.array::<u8>(provenance.compiler_build_id.len()),
    )?;
    let build_id_grant = metadata.grant_layout(Layout::for_value::<[u8]>(
        provenance.compiler_build_id.as_bytes(),
    ))?;
    let compiler_build_id = provenance.compiler_build_id.clone();
    metadata.finish(build_id_grant.bytes())?;

    let mut candidate = PortablePublicationCandidate {
        canonical_artifact: canonical_artifact.into_host(),
        source_map: source_map.into_host(),
        semantic_diff: semantic_diff.into_host(),
        network_revision,
        compiler_build_id,
        source_collection_digest_version: SOURCE_COLLECTION_DIGEST_VERSION_V1,
        source_collection_digest,
        expected_semantic_diff_base,
        resource_report: PortableEmissionResourceReport::default(),
    };
    build_id_grant.transfer_to_host();
    candidate.resource_report = ledger.close()?.into();
    Ok(candidate)
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

/// 测试用：改写拥有型 LFCA 对象第 8 节的声明网络修订。
#[cfg(test)]
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
