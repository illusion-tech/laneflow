//! `CompilationOutput` 到 LFCA/LFSM/LFSD 的原子可移植候选发射。
//!
//! 本模块拥有编译器私有 LIR/source-map 语义投影、摘要和跨对象绑定。线格式的结构、
//! 编码和值域预检仍只由 `laneflow-format` 提供；文件系统安装、LFCP 与独立验证收据不在
//! 本切片内。

use std::collections::BTreeMap;
use std::fmt::Write as _;

use laneflow_format::{
    FieldWriteInputV1, FieldWriteValueV1, FormatError, FormatLimits, ObjectWriteInputV1,
    RegistryCheckedFieldValue, RegistryCheckedObjectView, RegistryCheckedOrdinalVectorView,
    RegistryCheckedRecordVectorView, RegistryCheckedRowView, RowWriteInputV1, SectionWriteInputV1,
    TableWriteInputV1, ValueCheckedObjectView, encode_object_v1, measure_object_v1,
    preflight_object_values_v1,
};
use laneflow_static_contract::{
    CANONICAL_ARTIFACT_FORMAT_VERSION, EntityKind, EntityKindMarker,
    FORMAT_HARD_MAX_CANDIDATE_STAGING_BYTES, IDENTITY_ENCODING_VERSION, IDENTITY_REGISTRY_REVISION,
    NETWORK_REVISION_DERIVATION_VERSION, NETWORK_REVISION_DOMAIN_PREFIX, Ordinal, OrdinalKind,
    PortableObjectKind, SECTION_FORMAT_VERSION_V1, StableId,
};
use sha2::{Digest, Sha256};

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

/// 可移植发射的显式规范 provenance。
///
/// v1 只允许调用方提供 canonical compiler build ID；来源集合、编译选项、几何档位与
/// emitter 版本全部由同一个 `CompilationOutput` 和冻结规则派生。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortableEmissionProvenanceV1 {
    compiler_build_id: Box<str>,
}

impl PortableEmissionProvenanceV1 {
    /// 建立一份已规范化的 v1 provenance。
    ///
    /// # Errors
    ///
    /// build ID 不是 1..=128-byte ASCII，首字符不是字母/数字，或其余字符不属于
    /// `[A-Za-z0-9._+@-]` 时失败。
    pub fn try_new(compiler_build_id: impl Into<Box<str>>) -> Result<Self, PortableEmissionError> {
        let compiler_build_id = compiler_build_id.into();
        let bytes = compiler_build_id.as_bytes();
        let first_is_valid = bytes
            .first()
            .is_some_and(|byte| byte.is_ascii_alphanumeric());
        let all_are_valid = bytes.iter().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'+' | b'@' | b'-')
        });
        if !(1..=128).contains(&bytes.len()) || !first_is_valid || !all_are_valid {
            return Err(PortableEmissionError::InvalidCompilerBuildId);
        }
        Ok(Self { compiler_build_id })
    }

    /// 返回 exact-byte 发射输入中的 canonical compiler build ID。
    #[must_use]
    pub fn compiler_build_id(&self) -> &str {
        &self.compiler_build_id
    }
}

/// 一份候选对象的不可覆盖计算绑定。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortableObjectCandidate {
    bytes: Box<[u8]>,
    digest: [u8; 32],
    object_key: Box<str>,
}

impl PortableObjectCandidate {
    /// 返回完整 exact bytes。
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// 返回从 exact bytes 重算的 SHA-256。
    #[must_use]
    pub const fn digest(&self) -> [u8; 32] {
        self.digest
    }

    /// 返回 exact `u64` byte length。
    #[must_use]
    pub fn byte_length(&self) -> u64 {
        u64::try_from(self.bytes.len()).expect("supported targets have at most 64-bit usize")
    }

    /// 返回唯一 `sha256/<64 lowercase hex>` object key。
    #[must_use]
    pub fn object_key(&self) -> &str {
        &self.object_key
    }
}

/// 同一次发射原子拥有的三对象未受信发布候选。
///
/// 取得本类型只证明 compiler emitter 已关闭三份 bytes、完成格式预检和内部绑定核对；
/// 它不是独立验证收据，也不授予发布或迁移权限。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortablePublicationCandidate {
    canonical_artifact: PortableObjectCandidate,
    source_map: PortableObjectCandidate,
    semantic_diff: PortableObjectCandidate,
    network_revision: [u8; 32],
}

/// LFSD 的显式 base 选择。
///
/// `Artifact` 只接受已经完成格式结构和值域预检的借用。该能力不证明跨表引用、身份闭包、
/// revision 或真实性；emitter 只把它用于诊断性差异，并在分类前额外执行 v1 contract 与
/// 跨修订身份冲突检查。
#[derive(Clone, Copy, Debug)]
pub enum PortableDiffBase<'a> {
    Genesis,
    Artifact(ValueCheckedObjectView<'a>),
}

impl PortablePublicationCandidate {
    #[must_use]
    pub const fn canonical_artifact(&self) -> &PortableObjectCandidate {
        &self.canonical_artifact
    }

    #[must_use]
    pub const fn source_map(&self) -> &PortableObjectCandidate {
        &self.source_map
    }

    #[must_use]
    pub const fn semantic_diff(&self) -> &PortableObjectCandidate {
        &self.semantic_diff
    }

    #[must_use]
    pub const fn network_revision(&self) -> [u8; 32] {
        self.network_revision
    }
}

/// 可移植候选发射失败。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortableEmissionError {
    InvalidCompilerBuildId,
    Format(FormatError),
    ArithmeticOverflow,
    CandidateStagingLimitExceeded { actual: u64, limit: u64 },
    InvalidDiffBaseKind,
    DiffBaseSemanticMismatch,
    UnsupportedSemanticContractTransition,
    CrossRevisionStableIdCollision,
    InternalBindingMismatch,
}

impl From<FormatError> for PortableEmissionError {
    fn from(value: FormatError) -> Self {
        Self::Format(value)
    }
}

#[derive(Clone, Debug, PartialEq)]
enum OwnedValue {
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    F32(f32),
    F64(f64),
    StableId128([u8; 16]),
    Sha256([u8; 32]),
    Utf8(Box<str>),
    Bytes(Box<[u8]>),
    OrdinalVectorU32(Box<[u32]>),
    RecordVector(Box<[OwnedRow]>),
    I32(i32),
}

#[derive(Clone, Debug, PartialEq)]
struct OwnedField {
    tag: u16,
    value: OwnedValue,
}

#[derive(Clone, Debug, PartialEq)]
struct OwnedRow {
    fields: Box<[OwnedField]>,
}

#[derive(Clone, Debug, PartialEq)]
struct OwnedTable {
    kind: u16,
    rows: Box<[OwnedRow]>,
}

#[derive(Clone, Debug, PartialEq)]
struct OwnedSection {
    kind: u16,
    tables: Box<[OwnedTable]>,
}

#[derive(Clone, Debug, PartialEq)]
struct OwnedObject {
    kind: PortableObjectKind,
    sections: Box<[OwnedSection]>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum LocationValue {
    Text {
        source_module_ordinal: u32,
        source_document_ordinal: u32,
        start_line: u32,
        start_column: u32,
        end_line: u32,
        end_column: u32,
    },
    RoadEditing {
        source_module_ordinal: u32,
        source_document_ordinal: u32,
        subject_kind: u8,
        module_namespace: Option<Box<str>>,
        entity_kind: Option<u16>,
        owner_local_keys: [Option<Box<str>>; 3],
        local_key: Option<Box<str>>,
        owner_kind: Option<u8>,
        relation_kind: Option<u8>,
        occurrence_kind: Option<u8>,
        occurrence_ordinal: Option<u32>,
        property_steps: Option<Box<[(u8, u16, u16)]>>,
        canvas_selection: Option<Box<str>>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StableSourceProjection {
    entity_kind: EntityKind,
    stable_id: [u8; 16],
    typed_ordinal: u32,
    primary: LocationValue,
    contributing: Vec<LocationValue>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnerLocalProjection {
    owner_entity_kind: EntityKind,
    owner_stable_id: [u8; 16],
    role: u8,
    local_index: u32,
    primary: LocationValue,
    contributing: Vec<LocationValue>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SpatialRangeProjection {
    owner_entity_kind: EntityKind,
    owner_stable_id: [u8; 16],
    role: u8,
    local_index: u32,
    point_start: u32,
    point_end_exclusive: u32,
    source_segment_ordinal: u32,
    source: LocationValue,
}

type RoadEditingAddressProjection = (Box<str>, Option<u16>, [Option<Box<str>>; 3], Box<str>);
type GeometryValues = BTreeMap<(EntityKind, [u8; 16]), Box<[u8]>>;
type RelationGroups = BTreeMap<(EntityKind, [u8; 16], u8), Vec<RelationTuple>>;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct RelationTuple {
    owner_entity_kind: EntityKind,
    owner_stable_id: [u8; 16],
    role: u8,
    local_index: u32,
    subject_entity_kind: EntityKind,
    subject_stable_id: [u8; 16],
}

fn field(tag: u16, value: OwnedValue) -> OwnedField {
    OwnedField { tag, value }
}

fn row(fields: impl IntoIterator<Item = OwnedField>) -> OwnedRow {
    OwnedRow {
        fields: fields.into_iter().collect(),
    }
}

fn table(kind: u16, rows: impl IntoIterator<Item = OwnedRow>) -> OwnedTable {
    OwnedTable {
        kind,
        rows: rows.into_iter().collect(),
    }
}

fn section(kind: u16, tables: impl IntoIterator<Item = OwnedTable>) -> OwnedSection {
    OwnedSection {
        kind,
        tables: tables.into_iter().collect(),
    }
}

fn borrow_primitive_value(value: &OwnedValue) -> FieldWriteValueV1<'_> {
    match value {
        OwnedValue::U8(value) => FieldWriteValueV1::U8(*value),
        OwnedValue::U16(value) => FieldWriteValueV1::U16(*value),
        OwnedValue::U32(value) => FieldWriteValueV1::U32(*value),
        OwnedValue::U64(value) => FieldWriteValueV1::U64(*value),
        OwnedValue::F32(value) => FieldWriteValueV1::F32(*value),
        OwnedValue::F64(value) => FieldWriteValueV1::F64(*value),
        OwnedValue::StableId128(value) => FieldWriteValueV1::StableId128(*value),
        OwnedValue::Sha256(value) => FieldWriteValueV1::Sha256(*value),
        OwnedValue::Utf8(value) => FieldWriteValueV1::Utf8(value),
        OwnedValue::Bytes(value) => FieldWriteValueV1::Bytes(value),
        OwnedValue::OrdinalVectorU32(value) => FieldWriteValueV1::OrdinalVectorU32(value),
        OwnedValue::I32(value) => FieldWriteValueV1::I32(*value),
        OwnedValue::RecordVector(_) => unreachable!("record vectors are lowered in a prior layer"),
    }
}

/// 把拥有型 compiler projection 临时降低为 `laneflow-format` 的零分配借用 writer 输入。
///
/// v1 只允许一层 RecordVector，因此各借用层按 nested fields -> nested rows -> top fields
/// -> top rows -> tables -> sections 的顺序建立；没有自引用拥有结构或泄漏分配。
fn encode_owned_object(
    object: &OwnedObject,
    limits: FormatLimits,
    already_staged_bytes: u64,
) -> Result<Box<[u8]>, PortableEmissionError> {
    let mut nested_field_groups = Vec::<Box<[FieldWriteInputV1<'_>]>>::new();
    for section in &object.sections {
        for table in &section.tables {
            for row in &table.rows {
                for field in &row.fields {
                    if let OwnedValue::RecordVector(rows) = &field.value {
                        for nested_row in rows {
                            let fields = nested_row
                                .fields
                                .iter()
                                .map(|field| FieldWriteInputV1 {
                                    tag: field.tag,
                                    value: borrow_primitive_value(&field.value),
                                })
                                .collect();
                            nested_field_groups.push(fields);
                        }
                    }
                }
            }
        }
    }

    let mut nested_field_index = 0_usize;
    let mut nested_row_groups = Vec::<Box<[RowWriteInputV1<'_>]>>::new();
    for section in &object.sections {
        for table in &section.tables {
            for row in &table.rows {
                for field in &row.fields {
                    if let OwnedValue::RecordVector(rows) = &field.value {
                        let start = nested_field_index;
                        nested_field_index += rows.len();
                        let row_inputs = nested_field_groups[start..nested_field_index]
                            .iter()
                            .map(|fields| RowWriteInputV1 { fields })
                            .collect();
                        nested_row_groups.push(row_inputs);
                    }
                }
            }
        }
    }

    let mut nested_row_index = 0_usize;
    let mut top_field_groups = Vec::<Box<[FieldWriteInputV1<'_>]>>::new();
    for section in &object.sections {
        for table in &section.tables {
            for row in &table.rows {
                let fields = row
                    .fields
                    .iter()
                    .map(|field| {
                        let value = match &field.value {
                            OwnedValue::RecordVector(_) => {
                                let rows = &nested_row_groups[nested_row_index];
                                nested_row_index += 1;
                                FieldWriteValueV1::RecordVector(rows)
                            }
                            value => borrow_primitive_value(value),
                        };
                        FieldWriteInputV1 {
                            tag: field.tag,
                            value,
                        }
                    })
                    .collect();
                top_field_groups.push(fields);
            }
        }
    }

    let mut top_field_index = 0_usize;
    let mut table_row_groups = Vec::<Box<[RowWriteInputV1<'_>]>>::new();
    for section in &object.sections {
        for table in &section.tables {
            let start = top_field_index;
            top_field_index += table.rows.len();
            let rows = top_field_groups[start..top_field_index]
                .iter()
                .map(|fields| RowWriteInputV1 { fields })
                .collect();
            table_row_groups.push(rows);
        }
    }

    let mut table_row_index = 0_usize;
    let mut section_table_groups = Vec::<Box<[TableWriteInputV1<'_>]>>::new();
    for section in &object.sections {
        let tables = section
            .tables
            .iter()
            .map(|table| {
                let rows = &table_row_groups[table_row_index];
                table_row_index += 1;
                TableWriteInputV1 {
                    kind: table.kind,
                    rows,
                }
            })
            .collect();
        section_table_groups.push(tables);
    }

    let sections: Box<[SectionWriteInputV1<'_>]> = object
        .sections
        .iter()
        .zip(&section_table_groups)
        .map(|(section, tables)| SectionWriteInputV1 {
            kind: section.kind,
            tables,
        })
        .collect();
    let input = ObjectWriteInputV1 {
        kind: object.kind,
        sections: &sections,
    };
    let length = measure_object_v1(input, limits)?;
    let candidate_length = already_staged_bytes
        .checked_add(length)
        .ok_or(PortableEmissionError::ArithmeticOverflow)?;
    if candidate_length > FORMAT_HARD_MAX_CANDIDATE_STAGING_BYTES {
        return Err(PortableEmissionError::CandidateStagingLimitExceeded {
            actual: candidate_length,
            limit: FORMAT_HARD_MAX_CANDIDATE_STAGING_BYTES,
        });
    }
    let output_length =
        usize::try_from(length).map_err(|_| PortableEmissionError::ArithmeticOverflow)?;
    let mut bytes = vec![0_u8; output_length];
    encode_object_v1(input, limits, &mut bytes)?;
    Ok(bytes.into_boxed_slice())
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn object_key(digest: [u8; 32]) -> Box<str> {
    let mut key = String::with_capacity(71);
    key.push_str("sha256/");
    for byte in digest {
        write!(&mut key, "{byte:02x}").expect("writing to String is infallible");
    }
    key.into_boxed_str()
}

fn close_object(bytes: Box<[u8]>) -> PortableObjectCandidate {
    let digest = sha256(&bytes);
    PortableObjectCandidate {
        bytes,
        digest,
        object_key: object_key(digest),
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
    let mut lfca = build_lfca(output, provenance, source_collection_digest, [0; 32]);
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
        canonical_artifact.byte_length(),
    )?);
    preflight_object_values_v1(source_map.bytes(), PortableObjectKind::SourceMap, limits)?;

    let lfsd = build_lfsd(output, base, network_revision, &canonical_artifact, limits)?;
    let staged_before_diff = canonical_artifact
        .byte_length()
        .checked_add(source_map.byte_length())
        .ok_or(PortableEmissionError::ArithmeticOverflow)?;
    let semantic_diff = close_object(encode_owned_object(&lfsd, limits, staged_before_diff)?);
    preflight_object_values_v1(
        semantic_diff.bytes(),
        PortableObjectKind::SemanticDiff,
        limits,
    )?;

    let total = canonical_artifact
        .byte_length()
        .checked_add(source_map.byte_length())
        .and_then(|value| value.checked_add(semantic_diff.byte_length()))
        .ok_or(PortableEmissionError::ArithmeticOverflow)?;
    if total > FORMAT_HARD_MAX_CANDIDATE_STAGING_BYTES {
        return Err(PortableEmissionError::CandidateStagingLimitExceeded {
            actual: total,
            limit: FORMAT_HARD_MAX_CANDIDATE_STAGING_BYTES,
        });
    }

    Ok(PortablePublicationCandidate {
        canonical_artifact,
        source_map,
        semantic_diff,
        network_revision,
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

fn network_revision(bytes: &[u8], limits: FormatLimits) -> Result<[u8; 32], PortableEmissionError> {
    let view = preflight_object_values_v1(bytes, PortableObjectKind::CanonicalArtifact, limits)?
        .registry_view();
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
    Ok(hasher.finalize().into())
}

fn set_lfca_network_revision(
    object: &mut OwnedObject,
    revision: [u8; 32],
) -> Result<(), PortableEmissionError> {
    let value = object
        .sections
        .get_mut(7)
        .and_then(|section| section.tables.get_mut(0))
        .and_then(|table| table.rows.get_mut(0))
        .and_then(|row| row.fields.get_mut(0))
        .ok_or(PortableEmissionError::InternalBindingMismatch)?;
    value.value = OwnedValue::Sha256(revision);
    Ok(())
}

fn road_editing_relation_code(value: crate::RoadEditingRelationKind) -> u8 {
    match value {
        crate::RoadEditingRelationKind::Import => 0,
        crate::RoadEditingRelationKind::CurveSegment => 1,
        crate::RoadEditingRelationKind::CorridorElement => 2,
        crate::RoadEditingRelationKind::RoadSectionAuthoringLane => 3,
        crate::RoadEditingRelationKind::LaneEdgeSuccessor => 4,
        crate::RoadEditingRelationKind::JunctionApproachEdge => 5,
        crate::RoadEditingRelationKind::JunctionInternalEdge => 6,
        crate::RoadEditingRelationKind::ManeuverPathInternalEdge => 7,
        crate::RoadEditingRelationKind::SignalControllerGroup => 8,
        crate::RoadEditingRelationKind::SignalControllerPhase => 9,
        crate::RoadEditingRelationKind::SignalPhaseState => 10,
        crate::RoadEditingRelationKind::AccessRuleParticipantClass => 11,
        crate::RoadEditingRelationKind::StaticRouteEdge => 12,
    }
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

fn source_language_code(value: crate::SourceLanguage) -> u16 {
    match value {
        crate::SourceLanguage::SyntheticDsl => 1,
        crate::SourceLanguage::RoadEditingSource => 3,
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

fn road_editing_table_code(value: crate::RoadEditingTableKind) -> u16 {
    match value {
        crate::RoadEditingTableKind::RoadEditingSource => 0,
        crate::RoadEditingTableKind::ModuleHeader => 1,
        crate::RoadEditingTableKind::Provenance => 2,
        crate::RoadEditingTableKind::LineSegment => 3,
        crate::RoadEditingTableKind::CubicBezierSegment => 4,
        crate::RoadEditingTableKind::CurveSegment => 5,
        crate::RoadEditingTableKind::CurveProgram => 6,
        crate::RoadEditingTableKind::RoadAlignment => 7,
        crate::RoadEditingTableKind::CorridorElement => 8,
        crate::RoadEditingTableKind::RoadCorridor => 9,
        crate::RoadEditingTableKind::RoadSection => 10,
        crate::RoadEditingTableKind::AuthoringLane => 11,
        crate::RoadEditingTableKind::LaneEdge => 12,
        crate::RoadEditingTableKind::Junction => 13,
        crate::RoadEditingTableKind::Movement => 14,
        crate::RoadEditingTableKind::ManeuverPath => 15,
        crate::RoadEditingTableKind::ManeuverGate => 16,
        crate::RoadEditingTableKind::WaitingZone => 17,
        crate::RoadEditingTableKind::StopLine => 18,
        crate::RoadEditingTableKind::SignalGroup => 19,
        crate::RoadEditingTableKind::SignalController => 20,
        crate::RoadEditingTableKind::SignalPhaseState => 21,
        crate::RoadEditingTableKind::SignalPhase => 22,
        crate::RoadEditingTableKind::ParkingArea => 23,
        crate::RoadEditingTableKind::ParkingLaneAnchor => 24,
        crate::RoadEditingTableKind::ParkingSpaceGeometry => 25,
        crate::RoadEditingTableKind::ParkingSpace => 26,
        crate::RoadEditingTableKind::LaneGroup => 27,
        crate::RoadEditingTableKind::FacilityBand => 28,
        crate::RoadEditingTableKind::ParticipantClass => 29,
        crate::RoadEditingTableKind::AccessRegulation => 30,
        crate::RoadEditingTableKind::AccessRule => 31,
        crate::RoadEditingTableKind::IidmVehicleProfile => 32,
        crate::RoadEditingTableKind::VehicleProfile => 33,
        crate::RoadEditingTableKind::StaticRoute => 34,
        crate::RoadEditingTableKind::CanonicalFrame => 35,
    }
}

fn road_editing_struct_code(value: crate::RoadEditingStructKind) -> u16 {
    match value {
        crate::RoadEditingStructKind::Digest256 => 0,
        crate::RoadEditingStructKind::OptionalU64 => 1,
        crate::RoadEditingStructKind::Vec3F64 => 2,
        crate::RoadEditingStructKind::LinearWidthProfile => 3,
    }
}

fn road_editing_union_code(value: crate::RoadEditingUnionKind) -> u16 {
    match value {
        crate::RoadEditingUnionKind::CurveSegmentGeometry => 0,
    }
}

fn property_steps(path: Option<&crate::RoadEditingPropertyPath>) -> Option<Box<[(u8, u16, u16)]>> {
    path.map(|path| {
        path.steps()
            .iter()
            .map(|step| match *step {
                crate::RoadEditingPropertyStep::TableField { table, field_id } => {
                    (0, road_editing_table_code(table), field_id)
                }
                crate::RoadEditingPropertyStep::StructMember {
                    structure,
                    member_id,
                } => (1, road_editing_struct_code(structure), u16::from(member_id)),
                crate::RoadEditingPropertyStep::UnionVariant {
                    union,
                    discriminant,
                } => (2, road_editing_union_code(union), u16::from(discriminant)),
            })
            .collect()
    })
}

type DocumentOrdinals<'a> = BTreeMap<&'a str, (u32, u32)>;

fn address_projection(
    address: crate::RoadEditingSourceAddress,
    context: &crate::RoadEditingLocationContext,
) -> RoadEditingAddressProjection {
    let mut owner_local_keys = [None, None, None];
    for (slot, key) in owner_local_keys
        .iter_mut()
        .zip(address.owner_local_keys(context))
    {
        *slot = Some(Box::from(key));
    }
    (
        Box::from(address.module_namespace(context)),
        address.entity_kind().map(EntityKind::code),
        owner_local_keys,
        Box::from(address.local_key(context)),
    )
}

fn location_value<'a>(
    view: crate::SourceLocationView<'a>,
    documents: &DocumentOrdinals<'a>,
) -> Result<LocationValue, PortableEmissionError> {
    let (source_module_ordinal, source_document_ordinal) = documents
        .get(view.source_document_key())
        .copied()
        .ok_or(PortableEmissionError::InternalBindingMismatch)?;
    match view {
        crate::SourceLocationView::Text { start, end, .. } => Ok(LocationValue::Text {
            source_module_ordinal,
            source_document_ordinal,
            start_line: start.line(),
            start_column: start.column(),
            end_line: end.line(),
            end_column: end.column(),
        }),
        crate::SourceLocationView::RoadEditing(location) => {
            if location.byte_range().is_some() {
                return Err(PortableEmissionError::InternalBindingMismatch);
            }
            let context = location.context();
            let mut module_namespace = None;
            let mut entity_kind = None;
            let mut owner_local_keys = [None, None, None];
            let mut local_key = None;
            let mut owner_kind = None;
            let mut relation_kind = None;
            let mut occurrence_kind = None;
            let mut occurrence_ordinal = None;
            let subject_kind = match *location.subject() {
                crate::RoadEditingSubject::ModuleHeader => 0,
                crate::RoadEditingSubject::RoadAlignment { address } => {
                    let (namespace, entity, owners, key) = address_projection(address, context);
                    module_namespace = Some(namespace);
                    entity_kind = entity;
                    owner_local_keys = owners;
                    local_key = Some(key);
                    1
                }
                crate::RoadEditingSubject::Declaration { address } => {
                    let (namespace, entity, owners, key) = address_projection(address, context);
                    module_namespace = Some(namespace);
                    entity_kind = entity;
                    owner_local_keys = owners;
                    local_key = Some(key);
                    2
                }
                crate::RoadEditingSubject::OwnerLocal {
                    owner,
                    relation,
                    occurrence,
                } => {
                    match owner {
                        crate::RoadEditingOwner::ModuleHeader => owner_kind = Some(0),
                        crate::RoadEditingOwner::Address(address) => {
                            owner_kind = Some(1);
                            let (namespace, entity, owners, key) =
                                address_projection(address, context);
                            module_namespace = Some(namespace);
                            entity_kind = entity;
                            owner_local_keys = owners;
                            local_key = Some(key);
                        }
                    }
                    relation_kind = Some(road_editing_relation_code(relation));
                    let (kind, ordinal) = match occurrence {
                        crate::RoadEditingRelationOccurrence::OrderedProductOrdinal(ordinal) => {
                            (0, ordinal)
                        }
                        crate::RoadEditingRelationOccurrence::CanonicalSetOrdinal(ordinal) => {
                            (1, ordinal)
                        }
                    };
                    occurrence_kind = Some(kind);
                    occurrence_ordinal = Some(ordinal);
                    3
                }
                crate::RoadEditingSubject::Wire { .. } => {
                    return Err(PortableEmissionError::InternalBindingMismatch);
                }
            };
            Ok(LocationValue::RoadEditing {
                source_module_ordinal,
                source_document_ordinal,
                subject_kind,
                module_namespace,
                entity_kind,
                owner_local_keys,
                local_key,
                owner_kind,
                relation_kind,
                occurrence_kind,
                occurrence_ordinal,
                property_steps: property_steps(location.property_path()),
                canvas_selection: location.canvas_selection().map(Box::from),
            })
        }
    }
}

fn location_ordinal(
    locations: &[LocationValue],
    location: &LocationValue,
) -> Result<u32, PortableEmissionError> {
    let index = locations
        .binary_search(location)
        .map_err(|_| PortableEmissionError::InternalBindingMismatch)?;
    u32::try_from(index).map_err(|_| PortableEmissionError::ArithmeticOverflow)
}

fn location_set_ordinals(
    locations: &[LocationValue],
    values: &[LocationValue],
) -> Result<Box<[u32]>, PortableEmissionError> {
    let mut values = values.to_vec();
    values.sort_unstable();
    values.dedup();
    values
        .iter()
        .map(|value| location_ordinal(locations, value))
        .collect()
}

fn location_row(ordinal: u32, location: &LocationValue) -> OwnedRow {
    match location {
        LocationValue::Text {
            source_module_ordinal,
            source_document_ordinal,
            start_line,
            start_column,
            end_line,
            end_column,
        } => row([
            field(1, OwnedValue::U32(ordinal)),
            field(2, OwnedValue::U8(0)),
            field(3, OwnedValue::U32(*source_module_ordinal)),
            field(4, OwnedValue::U32(*source_document_ordinal)),
            field(5, OwnedValue::U32(*start_line)),
            field(6, OwnedValue::U32(*start_column)),
            field(7, OwnedValue::U32(*end_line)),
            field(8, OwnedValue::U32(*end_column)),
        ]),
        LocationValue::RoadEditing {
            source_module_ordinal,
            source_document_ordinal,
            subject_kind,
            module_namespace,
            entity_kind,
            owner_local_keys,
            local_key,
            owner_kind,
            relation_kind,
            occurrence_kind,
            occurrence_ordinal,
            property_steps,
            canvas_selection,
        } => {
            let mut fields = vec![
                field(1, OwnedValue::U32(ordinal)),
                field(2, OwnedValue::U8(1)),
                field(3, OwnedValue::U32(*source_module_ordinal)),
                field(4, OwnedValue::U32(*source_document_ordinal)),
                field(9, OwnedValue::U8(*subject_kind)),
            ];
            if let Some(value) = module_namespace {
                fields.push(field(10, OwnedValue::Utf8(value.clone())));
            }
            if let Some(value) = entity_kind {
                fields.push(field(11, OwnedValue::U16(*value)));
            }
            for (tag, value) in (12..=14).zip(owner_local_keys) {
                if let Some(value) = value {
                    fields.push(field(tag, OwnedValue::Utf8(value.clone())));
                }
            }
            if let Some(value) = local_key {
                fields.push(field(15, OwnedValue::Utf8(value.clone())));
            }
            if let Some(value) = owner_kind {
                fields.push(field(16, OwnedValue::U8(*value)));
            }
            if let Some(value) = relation_kind {
                fields.push(field(17, OwnedValue::U8(*value)));
            }
            if let Some(value) = occurrence_kind {
                fields.push(field(18, OwnedValue::U8(*value)));
            }
            if let Some(value) = occurrence_ordinal {
                fields.push(field(19, OwnedValue::U32(*value)));
            }
            if let Some(steps) = property_steps {
                let rows = steps
                    .iter()
                    .map(|(kind, container, member)| {
                        row([
                            field(1, OwnedValue::U8(*kind)),
                            field(2, OwnedValue::U16(*container)),
                            field(3, OwnedValue::U16(*member)),
                        ])
                    })
                    .collect();
                fields.push(field(20, OwnedValue::RecordVector(rows)));
            }
            if let Some(value) = canvas_selection {
                fields.push(field(21, OwnedValue::Utf8(value.clone())));
            }
            row(fields)
        }
    }
}

fn expected_stable_source_keys(lir: &crate::lir::LirUnit) -> Vec<(EntityKind, [u8; 16], u32)> {
    let mut keys = Vec::new();
    macro_rules! append {
        ($kind:expr, $records:expr) => {
            keys.extend($records.iter().map(|record| {
                (
                    $kind,
                    stable_id_bytes(record.stable_id),
                    record.ordinal.raw(),
                )
            }));
        };
    }
    append!(EntityKind::RoadCorridor, lir.road_corridors);
    append!(EntityKind::RoadSection, lir.road_sections);
    append!(EntityKind::AuthoringLane, lir.authoring_lanes);
    append!(EntityKind::LaneEdge, lir.lane_edges);
    append!(EntityKind::Junction, lir.junctions);
    append!(EntityKind::Movement, lir.movements);
    append!(EntityKind::ManeuverPath, lir.maneuver_paths);
    append!(EntityKind::ManeuverGate, lir.maneuver_gates);
    append!(EntityKind::WaitingZone, lir.waiting_zones);
    append!(EntityKind::StopLine, lir.stop_lines);
    append!(EntityKind::SignalGroup, lir.signal_groups);
    append!(EntityKind::SignalController, lir.signal_controllers);
    append!(EntityKind::SignalPhase, lir.signal_phases);
    append!(EntityKind::ParkingArea, lir.parking_areas);
    append!(EntityKind::ParkingSpace, lir.parking_spaces);
    append!(EntityKind::LaneGroup, lir.lane_groups);
    append!(EntityKind::FacilityBand, lir.facility_bands);
    append!(EntityKind::ParticipantClass, lir.participant_classes);
    append!(EntityKind::AccessRule, lir.access_rules);
    append!(EntityKind::VehicleProfile, lir.vehicle_profiles);
    append!(EntityKind::StaticRoute, lir.static_routes);
    append!(EntityKind::CanonicalFrame, lir.canonical_frames);
    keys.sort_unstable();
    keys
}

fn expected_owner_local_source_keys(
    lir: &crate::lir::LirUnit,
) -> Vec<(EntityKind, [u8; 16], u8, u32)> {
    let mut keys: Vec<_> = canonical_relation_tuples(lir)
        .into_iter()
        .map(|relation| {
            (
                relation.owner_entity_kind,
                relation.owner_stable_id,
                relation.role,
                relation.local_index,
            )
        })
        .collect();
    for phase in &lir.signal_phases {
        for (local_index, _) in lir.signal_phase_states[phase.states.as_usize_range()]
            .iter()
            .enumerate()
        {
            keys.push((
                EntityKind::SignalPhase,
                stable_id_bytes(phase.stable_id),
                19,
                u32::try_from(local_index).expect("compile limits cap relation counts at u32"),
            ));
        }
    }
    let mut next_lane_geometry_index = vec![0_u32; lir.canonical_frames.len()];
    for geometry in &lir.lane_edge_geometries {
        let frame = geometry.canonical_frame;
        let local_index = next_lane_geometry_index[frame.index()];
        next_lane_geometry_index[frame.index()] += 1;
        keys.push((
            EntityKind::CanonicalFrame,
            stable_id_bytes(lir.canonical_frames[frame.index()].stable_id),
            28,
            local_index,
        ));
    }
    let mut next_facility_geometry_index = vec![0_u32; lir.canonical_frames.len()];
    for geometry in &lir.facility_band_geometries {
        let frame = geometry.canonical_frame;
        let local_index = next_facility_geometry_index[frame.index()];
        next_facility_geometry_index[frame.index()] += 1;
        keys.push((
            EntityKind::CanonicalFrame,
            stable_id_bytes(lir.canonical_frames[frame.index()].stable_id),
            29,
            local_index,
        ));
    }
    keys.sort_unstable();
    keys
}

fn build_lfsm(
    output: &CompilationOutput,
    provenance: &PortableEmissionProvenanceV1,
    source_collection_digest: [u8; 32],
    network_revision: [u8; 32],
    artifact: &PortableObjectCandidate,
) -> Result<OwnedObject, PortableEmissionError> {
    let source_map = output.source_map_input();
    let modules: Vec<_> = source_map.source_modules().collect();
    let module_ordinals: BTreeMap<_, _> = modules
        .iter()
        .enumerate()
        .map(|(ordinal, module)| {
            Ok((
                module.authoring_namespace_id(),
                u32::try_from(ordinal).map_err(|_| PortableEmissionError::ArithmeticOverflow)?,
            ))
        })
        .collect::<Result<_, PortableEmissionError>>()?;
    let mut documents: Vec<_> = source_map
        .source_documents()
        .map(|document| {
            let module = module_ordinals
                .get(document.authoring_namespace_id())
                .copied()
                .ok_or(PortableEmissionError::InternalBindingMismatch)?;
            Ok((module, document.source_document_key(), document))
        })
        .collect::<Result<_, PortableEmissionError>>()?;
    documents.sort_unstable_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.as_bytes().cmp(right.1.as_bytes()))
    });
    let document_ordinals: DocumentOrdinals<'_> = documents
        .iter()
        .enumerate()
        .map(|(ordinal, (module, key, _))| {
            Ok((
                *key,
                (
                    *module,
                    u32::try_from(ordinal)
                        .map_err(|_| PortableEmissionError::ArithmeticOverflow)?,
                ),
            ))
        })
        .collect::<Result<_, PortableEmissionError>>()?;
    let lir = output.lir().unit();

    let mut stable_sources = Vec::new();
    macro_rules! append_stable_sources {
        ($kind:expr, $iterator:expr) => {
            for source in $iterator {
                stable_sources.push(StableSourceProjection {
                    entity_kind: $kind,
                    stable_id: stable_id_bytes(source.stable_id()),
                    typed_ordinal: source.ordinal().raw(),
                    primary: location_value(source.primary_source(), &document_ordinals)?,
                    contributing: source
                        .contributing_sources()
                        .map(|location| location_value(location, &document_ordinals))
                        .collect::<Result<_, _>>()?,
                });
            }
        };
    }
    append_stable_sources!(EntityKind::RoadCorridor, source_map.road_corridor_sources());
    append_stable_sources!(EntityKind::RoadSection, source_map.road_section_sources());
    append_stable_sources!(
        EntityKind::AuthoringLane,
        source_map.authoring_lane_sources()
    );
    append_stable_sources!(EntityKind::LaneEdge, source_map.lane_edge_sources());
    append_stable_sources!(EntityKind::Junction, source_map.junction_sources());
    append_stable_sources!(EntityKind::Movement, source_map.movement_sources());
    append_stable_sources!(EntityKind::ManeuverPath, source_map.maneuver_path_sources());
    append_stable_sources!(EntityKind::ManeuverGate, source_map.maneuver_gate_sources());
    append_stable_sources!(EntityKind::WaitingZone, source_map.waiting_zone_sources());
    append_stable_sources!(EntityKind::StopLine, source_map.stop_line_sources());
    append_stable_sources!(EntityKind::SignalGroup, source_map.signal_group_sources());
    append_stable_sources!(
        EntityKind::SignalController,
        source_map.signal_controller_sources()
    );
    append_stable_sources!(EntityKind::SignalPhase, source_map.signal_phase_sources());
    append_stable_sources!(EntityKind::ParkingArea, source_map.parking_area_sources());
    append_stable_sources!(EntityKind::ParkingSpace, source_map.parking_space_sources());
    append_stable_sources!(EntityKind::LaneGroup, source_map.lane_group_sources());
    append_stable_sources!(EntityKind::FacilityBand, source_map.facility_band_sources());
    append_stable_sources!(
        EntityKind::ParticipantClass,
        source_map.participant_class_sources()
    );
    append_stable_sources!(EntityKind::AccessRule, source_map.access_rule_sources());
    append_stable_sources!(
        EntityKind::VehicleProfile,
        source_map.vehicle_profile_sources()
    );
    append_stable_sources!(EntityKind::StaticRoute, source_map.static_route_sources());
    append_stable_sources!(
        EntityKind::CanonicalFrame,
        source_map.canonical_frame_sources()
    );
    stable_sources.sort_unstable_by_key(|source| {
        (source.entity_kind, source.stable_id, source.typed_ordinal)
    });
    let actual_stable_keys: Vec<_> = stable_sources
        .iter()
        .map(|source| (source.entity_kind, source.stable_id, source.typed_ordinal))
        .collect();
    if actual_stable_keys != expected_stable_source_keys(lir) {
        return Err(PortableEmissionError::InternalBindingMismatch);
    }

    let mut owner_local_sources = Vec::new();
    let mut spatial_ranges = Vec::new();
    let internal_edges: Vec<bool> = (0..lir.lane_edges.len())
        .map(|ordinal| {
            lir.junction_internal_edges
                .binary_search_by_key(
                    &u32::try_from(ordinal).expect("compile limits cap entity counts at u32"),
                    |entry| entry.edge.raw(),
                )
                .is_ok()
        })
        .collect();
    let mut next_successor_index_by_owner = vec![0_u32; lir.lane_edges.len()];
    for source in source_map.lane_edge_successor_sources() {
        let owner = source.owner_ordinal();
        let edge = &lir.lane_edges[owner.index()];
        let target = lir.lane_edge_successors[edge.successors.as_usize_range()][usize::try_from(
            source.local_index(),
        )
        .expect("supported compiler targets can index a validated local relation")];
        if internal_edges[owner.index()] || internal_edges[target.index()] {
            continue;
        }
        let local_index = next_successor_index_by_owner[owner.index()];
        next_successor_index_by_owner[owner.index()] += 1;
        owner_local_sources.push(OwnerLocalProjection {
            owner_entity_kind: EntityKind::LaneEdge,
            owner_stable_id: stable_id_bytes(source.owner_stable_id()),
            role: source_relation_role_code(source.role()),
            local_index,
            primary: location_value(source.primary_source(), &document_ordinals)?,
            contributing: source
                .contributing_sources()
                .map(|location| location_value(location, &document_ordinals))
                .collect::<Result<_, _>>()?,
        });
    }

    macro_rules! push_owner_local {
        ($kind:expr, $stable_id:expr, $source:expr) => {{
            let source = $source;
            owner_local_sources.push(OwnerLocalProjection {
                owner_entity_kind: $kind,
                owner_stable_id: stable_id_bytes($stable_id),
                role: source_relation_role_code(source.role()),
                local_index: source.local_index(),
                primary: location_value(source.primary_source(), &document_ordinals)?,
                contributing: source
                    .contributing_sources()
                    .map(|location| location_value(location, &document_ordinals))
                    .collect::<Result<_, _>>()?,
            });
        }};
    }
    for source in source_map.cross_section_relation_sources() {
        match source.owner() {
            crate::CrossSectionRelationOwner::RoadCorridor(_, id) => {
                push_owner_local!(EntityKind::RoadCorridor, id, source)
            }
            crate::CrossSectionRelationOwner::RoadSection(_, id) => {
                push_owner_local!(EntityKind::RoadSection, id, source)
            }
            crate::CrossSectionRelationOwner::AuthoringLane(_, id) => {
                push_owner_local!(EntityKind::AuthoringLane, id, source)
            }
            crate::CrossSectionRelationOwner::LaneGroup(_, id) => {
                push_owner_local!(EntityKind::LaneGroup, id, source)
            }
        }
    }
    for source in source_map.junction_relation_sources() {
        match source.owner() {
            crate::JunctionRelationOwner::Junction(_, id) => {
                push_owner_local!(EntityKind::Junction, id, source)
            }
            crate::JunctionRelationOwner::Movement(_, id) => {
                push_owner_local!(EntityKind::Movement, id, source)
            }
            crate::JunctionRelationOwner::ManeuverPath(_, id) => {
                push_owner_local!(EntityKind::ManeuverPath, id, source)
            }
            crate::JunctionRelationOwner::StopLine(_, id) => {
                push_owner_local!(EntityKind::StopLine, id, source)
            }
        }
    }
    for source in source_map.signal_relation_sources() {
        match source.owner() {
            crate::SignalRelationOwner::SignalController(_, id) => {
                push_owner_local!(EntityKind::SignalController, id, source)
            }
            crate::SignalRelationOwner::SignalPhase(_, id) => {
                push_owner_local!(EntityKind::SignalPhase, id, source)
            }
            crate::SignalRelationOwner::ManeuverGate(_, id) => {
                push_owner_local!(EntityKind::ManeuverGate, id, source)
            }
        }
    }
    for source in source_map.parking_relation_sources() {
        push_owner_local!(EntityKind::ParkingSpace, source.owner_stable_id(), source);
    }
    for source in source_map.access_relation_sources() {
        match source.owner() {
            crate::AccessRelationOwner::ParticipantClass(_, id) => {
                push_owner_local!(EntityKind::ParticipantClass, id, source)
            }
            crate::AccessRelationOwner::VehicleProfile(_, id) => {
                push_owner_local!(EntityKind::VehicleProfile, id, source)
            }
            crate::AccessRelationOwner::AccessRule(_, id) => {
                push_owner_local!(EntityKind::AccessRule, id, source)
            }
        }
    }
    for source in source_map.route_relation_sources() {
        push_owner_local!(EntityKind::StaticRoute, source.owner_stable_id(), source);
    }
    for source in source_map.spatial_relation_sources() {
        let owner_entity_kind = EntityKind::CanonicalFrame;
        let owner_stable_id = stable_id_bytes(source.owner_stable_id());
        let role = source_relation_role_code(source.role());
        let local_index = source.local_index();
        let primary = location_value(source.primary_source(), &document_ordinals)?;
        let contributing = source
            .contributing_sources()
            .map(|location| location_value(location, &document_ordinals))
            .collect::<Result<Vec<_>, _>>()?;
        for range in source.geometry_source_ranges() {
            let points = range.point_range();
            spatial_ranges.push(SpatialRangeProjection {
                owner_entity_kind,
                owner_stable_id,
                role,
                local_index,
                point_start: points.start,
                point_end_exclusive: points.end,
                source_segment_ordinal: range.source_segment_ordinal(),
                source: location_value(range.source(), &document_ordinals)?,
            });
        }
        owner_local_sources.push(OwnerLocalProjection {
            owner_entity_kind,
            owner_stable_id,
            role,
            local_index,
            primary,
            contributing,
        });
    }
    owner_local_sources.sort_unstable_by_key(|source| {
        (
            source.owner_entity_kind,
            source.owner_stable_id,
            source.role,
            source.local_index,
        )
    });
    let actual_owner_local_keys: Vec<_> = owner_local_sources
        .iter()
        .map(|source| {
            (
                source.owner_entity_kind,
                source.owner_stable_id,
                source.role,
                source.local_index,
            )
        })
        .collect();
    if actual_owner_local_keys != expected_owner_local_source_keys(lir) {
        return Err(PortableEmissionError::InternalBindingMismatch);
    }
    spatial_ranges.sort_unstable_by_key(|source| {
        (
            source.owner_entity_kind,
            source.owner_stable_id,
            source.role,
            source.local_index,
            source.point_start,
        )
    });

    let module_source_views: Vec<_> = source_map.source_module_sources().collect();
    if module_source_views.len() != modules.len() {
        return Err(PortableEmissionError::InternalBindingMismatch);
    }
    let mut locations = Vec::new();
    for source in &module_source_views {
        locations.push(location_value(source.primary_source(), &document_ordinals)?);
    }
    for source in &stable_sources {
        locations.push(source.primary.clone());
        locations.extend(source.contributing.iter().cloned());
    }
    for source in &owner_local_sources {
        locations.push(source.primary.clone());
        locations.extend(source.contributing.iter().cloned());
    }
    locations.extend(spatial_ranges.iter().map(|range| range.source.clone()));
    locations.sort_unstable();
    locations.dedup();

    let source_module_rows = module_source_views
        .iter()
        .enumerate()
        .map(|(ordinal, source)| {
            let descriptor = source.descriptor();
            let mut fields = vec![
                field(
                    1,
                    OwnedValue::U32(
                        u32::try_from(ordinal).expect("compile limits cap source modules at u32"),
                    ),
                ),
                field(
                    2,
                    OwnedValue::Utf8(Box::from(descriptor.authoring_namespace_id())),
                ),
                field(
                    3,
                    OwnedValue::U16(source_language_code(descriptor.source_language())),
                ),
                field(
                    4,
                    OwnedValue::Sha256(*descriptor.source_document_set_digest()),
                ),
                field(
                    5,
                    OwnedValue::U32(descriptor.source_document_set_digest_version()),
                ),
                field(6, OwnedValue::U32(descriptor.frontend_version())),
                field(7, OwnedValue::Sha256(*descriptor.frontend_options_digest())),
                field(
                    8,
                    OwnedValue::Utf8(Box::from(descriptor.generator_build_id())),
                ),
                field(
                    9,
                    OwnedValue::Sha256(*descriptor.parameters_and_inputs_digest()),
                ),
            ];
            if let Some(seed) = descriptor.random_seed() {
                fields.push(field(10, OwnedValue::U64(seed)));
            }
            fields.push(field(
                11,
                OwnedValue::Utf8(Box::from(descriptor.provenance())),
            ));
            let imports = descriptor
                .imports()
                .map(|namespace| row([field(1, OwnedValue::Utf8(Box::from(namespace)))]))
                .collect();
            fields.push(field(12, OwnedValue::RecordVector(imports)));
            let primary = location_value(source.primary_source(), &document_ordinals)?;
            fields.push(field(
                13,
                OwnedValue::U32(location_ordinal(&locations, &primary)?),
            ));
            Ok(row(fields))
        })
        .collect::<Result<Vec<_>, PortableEmissionError>>()?;
    let source_document_rows =
        documents
            .iter()
            .enumerate()
            .map(|(ordinal, (module_ordinal, _, document))| {
                let mut fields = vec![
                    field(
                        1,
                        OwnedValue::U32(
                            u32::try_from(ordinal)
                                .expect("compile limits cap source documents at u32"),
                        ),
                    ),
                    field(2, OwnedValue::U32(*module_ordinal)),
                    field(
                        3,
                        OwnedValue::Utf8(Box::from(document.source_document_key())),
                    ),
                    field(4, OwnedValue::Sha256(*document.source_document_digest())),
                    field(5, OwnedValue::U32(document.source_record_byte_len())),
                ];
                if let Some(display_source) = document.origin().display_source() {
                    fields.push(field(6, OwnedValue::Utf8(Box::from(display_source))));
                }
                row(fields)
            });
    let source_location_rows = locations.iter().enumerate().map(|(ordinal, location)| {
        location_row(
            u32::try_from(ordinal).expect("compile limits cap source locations at u32"),
            location,
        )
    });
    let stable_source_rows = stable_sources
        .iter()
        .map(|source| {
            Ok(row([
                field(1, OwnedValue::U16(source.entity_kind.code())),
                field(2, OwnedValue::StableId128(source.stable_id)),
                field(3, OwnedValue::U32(source.typed_ordinal)),
                field(
                    4,
                    OwnedValue::U32(location_ordinal(&locations, &source.primary)?),
                ),
                field(
                    5,
                    OwnedValue::OrdinalVectorU32(location_set_ordinals(
                        &locations,
                        &source.contributing,
                    )?),
                ),
            ]))
        })
        .collect::<Result<Vec<_>, PortableEmissionError>>()?;
    let owner_local_rows = owner_local_sources
        .iter()
        .map(|source| {
            Ok(row([
                field(1, OwnedValue::U16(source.owner_entity_kind.code())),
                field(2, OwnedValue::StableId128(source.owner_stable_id)),
                field(3, OwnedValue::U8(source.role)),
                field(4, OwnedValue::U32(source.local_index)),
                field(
                    5,
                    OwnedValue::U32(location_ordinal(&locations, &source.primary)?),
                ),
                field(
                    6,
                    OwnedValue::OrdinalVectorU32(location_set_ordinals(
                        &locations,
                        &source.contributing,
                    )?),
                ),
            ]))
        })
        .collect::<Result<Vec<_>, PortableEmissionError>>()?;
    let spatial_range_rows = spatial_ranges
        .iter()
        .map(|source| {
            Ok(row([
                field(1, OwnedValue::U16(source.owner_entity_kind.code())),
                field(2, OwnedValue::StableId128(source.owner_stable_id)),
                field(3, OwnedValue::U8(source.role)),
                field(4, OwnedValue::U32(source.local_index)),
                field(5, OwnedValue::U32(source.point_start)),
                field(6, OwnedValue::U32(source.point_end_exclusive)),
                field(7, OwnedValue::U32(source.source_segment_ordinal)),
                field(
                    8,
                    OwnedValue::U32(location_ordinal(&locations, &source.source)?),
                ),
            ]))
        })
        .collect::<Result<Vec<_>, PortableEmissionError>>()?;
    let derived_rows = owner_local_sources
        .iter()
        .filter(|source| matches!(source.role, 9 | 14 | 15 | 16))
        .map(|source| {
            let mut source_locations = source.contributing.clone();
            source_locations.push(source.primary.clone());
            let constraint_version = if source.role == 9 {
                CONSTRAINT_CONTRACT_VERSION_V1
            } else {
                STATIC_EXECUTION_CONTRACT_VERSION_V1
            };
            Ok(row([
                field(1, OwnedValue::U16(source.owner_entity_kind.code())),
                field(2, OwnedValue::StableId128(source.owner_stable_id)),
                field(3, OwnedValue::U8(source.role)),
                field(4, OwnedValue::U32(source.local_index)),
                field(5, OwnedValue::U16(1)),
                field(6, OwnedValue::U16(constraint_version)),
                field(
                    7,
                    OwnedValue::OrdinalVectorU32(location_set_ordinals(
                        &locations,
                        &source_locations,
                    )?),
                ),
            ]))
        })
        .collect::<Result<Vec<_>, PortableEmissionError>>()?;

    Ok(OwnedObject {
        kind: PortableObjectKind::SourceMap,
        sections: vec![
            section(
                1,
                [table(
                    1,
                    [row([
                        field(1, OwnedValue::U16(NETWORK_REVISION_DERIVATION_VERSION)),
                        field(2, OwnedValue::Sha256(network_revision)),
                        field(3, OwnedValue::U16(CANONICAL_ARTIFACT_FORMAT_VERSION)),
                        field(4, OwnedValue::Sha256(artifact.digest())),
                        field(5, OwnedValue::U64(artifact.byte_length())),
                        field(6, OwnedValue::Utf8(provenance.compiler_build_id.clone())),
                        field(7, OwnedValue::U16(SOURCE_COLLECTION_DIGEST_VERSION_V1)),
                        field(8, OwnedValue::Sha256(source_collection_digest)),
                    ])],
                )],
            ),
            section(
                2,
                [
                    table(1, source_module_rows),
                    table(2, source_document_rows),
                    table(3, source_location_rows),
                ],
            ),
            section(3, [table(1, stable_source_rows)]),
            section(
                4,
                [table(1, owner_local_rows), table(2, spatial_range_rows)],
            ),
            section(5, [table(1, derived_rows)]),
        ]
        .into_boxed_slice(),
    })
}

fn build_genesis_lfsd(
    output: &CompilationOutput,
    network_revision: [u8; 32],
    artifact: &PortableObjectCandidate,
    limits: FormatLimits,
) -> Result<OwnedObject, PortableEmissionError> {
    let target = preflight_object_values_v1(
        artifact.bytes(),
        PortableObjectKind::CanonicalArtifact,
        limits,
    )?
    .registry_view();
    let entity_changes = genesis_entity_changes(target)?;
    let relation_changes = genesis_relation_changes(output.lir().unit());
    let geometry_changes = genesis_geometry_changes(output.lir().unit(), target)?;
    let spatial_presence = target
        .section(4)
        .and_then(|section| section.table(0))
        .and_then(|table| table.row(0))
        .ok_or(PortableEmissionError::InternalBindingMismatch)?
        .bytes()
        .to_vec()
        .into_boxed_slice();
    Ok(OwnedObject {
        kind: PortableObjectKind::SemanticDiff,
        sections: vec![
            section(
                1,
                [table(
                    1,
                    [row([
                        field(1, OwnedValue::U8(0)),
                        field(2, OwnedValue::U16(0)),
                        field(3, OwnedValue::Sha256([0; 32])),
                        field(4, OwnedValue::Sha256([0; 32])),
                        field(5, OwnedValue::U64(0)),
                        field(6, OwnedValue::U16(NETWORK_REVISION_DERIVATION_VERSION)),
                        field(7, OwnedValue::Sha256(network_revision)),
                        field(8, OwnedValue::Sha256(artifact.digest())),
                        field(9, OwnedValue::U64(artifact.byte_length())),
                    ])],
                )],
            ),
            section(2, [table(1, entity_changes)]),
            section(3, [table(1, relation_changes)]),
            section(4, [table(1, geometry_changes)]),
            section(5, [table(1, [])]),
            section(
                6,
                [table(
                    1,
                    [row([
                        field(1, OwnedValue::U8(0)),
                        field(3, OwnedValue::Bytes(spatial_presence)),
                    ])],
                )],
            ),
        ]
        .into_boxed_slice(),
    })
}

fn build_lfsd(
    output: &CompilationOutput,
    base: PortableDiffBase<'_>,
    network_revision: [u8; 32],
    artifact: &PortableObjectCandidate,
    limits: FormatLimits,
) -> Result<OwnedObject, PortableEmissionError> {
    match base {
        PortableDiffBase::Genesis => build_genesis_lfsd(output, network_revision, artifact, limits),
        PortableDiffBase::Artifact(base) => {
            build_artifact_lfsd(base, network_revision, artifact, limits)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct IdentityRecord {
    entity_kind: EntityKind,
    typed_ordinal: u32,
    canonical_fields: Box<[u8]>,
}

#[derive(Clone, Copy, Debug)]
struct EntityRecord<'a> {
    row: RegistryCheckedRowView<'a>,
}

struct ArtifactIndex<'a> {
    view: RegistryCheckedObjectView<'a>,
    identities: BTreeMap<[u8; 16], IdentityRecord>,
    entities: BTreeMap<(EntityKind, [u8; 16]), EntityRecord<'a>>,
    ordinal_stable_ids: BTreeMap<(EntityKind, u32), [u8; 16]>,
}

impl<'a> ArtifactIndex<'a> {
    fn build(
        view: RegistryCheckedObjectView<'a>,
        mismatch: PortableEmissionError,
    ) -> Result<Self, PortableEmissionError> {
        let identity_table = view
            .section(1)
            .and_then(|section| section.table(0))
            .ok_or(mismatch)?;
        let mut identities = BTreeMap::new();
        let mut identity_ordinals = BTreeMap::new();
        for ordinal in 0..identity_table.row_count() {
            let identity = identity_table.row(ordinal).ok_or(mismatch)?;
            let entity_kind =
                EntityKind::from_code(checked_u16_with(identity, 1, mismatch)?).ok_or(mismatch)?;
            let typed_ordinal = checked_u32_with(identity, 2, mismatch)?;
            let stable_id = checked_stable_id_with(identity, 3, mismatch)?;
            let canonical_fields = identity
                .field_by_tag(4)
                .ok_or(mismatch)?
                .value_bytes()
                .to_vec()
                .into_boxed_slice();
            if identities
                .insert(
                    stable_id,
                    IdentityRecord {
                        entity_kind,
                        typed_ordinal,
                        canonical_fields,
                    },
                )
                .is_some()
                || identity_ordinals
                    .insert((entity_kind, typed_ordinal), stable_id)
                    .is_some()
            {
                return Err(mismatch);
            }
        }

        let entity_section = view.section(2).ok_or(mismatch)?;
        let mut entities = BTreeMap::new();
        let mut ordinal_stable_ids = BTreeMap::new();
        for (table_ordinal, entity_kind) in EntityKind::ALL.into_iter().enumerate() {
            let table_ordinal = u32::try_from(table_ordinal)
                .map_err(|_| PortableEmissionError::ArithmeticOverflow)?;
            let entity_table = entity_section.table(table_ordinal).ok_or(mismatch)?;
            for row_ordinal in 0..entity_table.row_count() {
                let entity = entity_table.row(row_ordinal).ok_or(mismatch)?;
                let typed_ordinal = checked_u32_with(entity, 1, mismatch)?;
                let stable_id = checked_stable_id_with(entity, 2, mismatch)?;
                if entities
                    .insert((entity_kind, stable_id), EntityRecord { row: entity })
                    .is_some()
                    || ordinal_stable_ids
                        .insert((entity_kind, typed_ordinal), stable_id)
                        .is_some()
                {
                    return Err(mismatch);
                }
            }
        }
        if identities.len() != entities.len() {
            return Err(mismatch);
        }
        for (stable_id, identity) in &identities {
            if identity_ordinals.get(&(identity.entity_kind, identity.typed_ordinal))
                != Some(stable_id)
                || ordinal_stable_ids.get(&(identity.entity_kind, identity.typed_ordinal))
                    != Some(stable_id)
                || !entities.contains_key(&(identity.entity_kind, *stable_id))
            {
                return Err(mismatch);
            }
        }

        Ok(Self {
            view,
            identities,
            entities,
            ordinal_stable_ids,
        })
    }

    fn stable_id(
        &self,
        entity_kind: EntityKind,
        typed_ordinal: u32,
        mismatch: PortableEmissionError,
    ) -> Result<[u8; 16], PortableEmissionError> {
        self.ordinal_stable_ids
            .get(&(entity_kind, typed_ordinal))
            .copied()
            .ok_or(mismatch)
    }
}

fn checked_u8_with(
    row: RegistryCheckedRowView<'_>,
    tag: u16,
    mismatch: PortableEmissionError,
) -> Result<u8, PortableEmissionError> {
    match row.field_by_tag(tag).ok_or(mismatch)?.value()? {
        RegistryCheckedFieldValue::U8(value) => Ok(value),
        _ => Err(mismatch),
    }
}

fn checked_u16_with(
    row: RegistryCheckedRowView<'_>,
    tag: u16,
    mismatch: PortableEmissionError,
) -> Result<u16, PortableEmissionError> {
    match row.field_by_tag(tag).ok_or(mismatch)?.value()? {
        RegistryCheckedFieldValue::U16(value) => Ok(value),
        _ => Err(mismatch),
    }
}

fn checked_u32_with(
    row: RegistryCheckedRowView<'_>,
    tag: u16,
    mismatch: PortableEmissionError,
) -> Result<u32, PortableEmissionError> {
    match row.field_by_tag(tag).ok_or(mismatch)?.value()? {
        RegistryCheckedFieldValue::U32(value) => Ok(value),
        _ => Err(mismatch),
    }
}

fn checked_stable_id_with(
    row: RegistryCheckedRowView<'_>,
    tag: u16,
    mismatch: PortableEmissionError,
) -> Result<[u8; 16], PortableEmissionError> {
    match row.field_by_tag(tag).ok_or(mismatch)?.value()? {
        RegistryCheckedFieldValue::StableId128(value) => Ok(value.into_bytes()),
        _ => Err(mismatch),
    }
}

fn checked_ordinal_vector_with(
    row: RegistryCheckedRowView<'_>,
    tag: u16,
    mismatch: PortableEmissionError,
) -> Result<RegistryCheckedOrdinalVectorView<'_>, PortableEmissionError> {
    match row.field_by_tag(tag).ok_or(mismatch)?.value()? {
        RegistryCheckedFieldValue::OrdinalVectorU32(value) => Ok(value),
        _ => Err(mismatch),
    }
}

fn checked_record_vector_with(
    row: RegistryCheckedRowView<'_>,
    tag: u16,
    mismatch: PortableEmissionError,
) -> Result<RegistryCheckedRecordVectorView<'_>, PortableEmissionError> {
    match row.field_by_tag(tag).ok_or(mismatch)?.value()? {
        RegistryCheckedFieldValue::RecordVector(value) => Ok(value),
        _ => Err(mismatch),
    }
}

fn singleton_row(
    view: RegistryCheckedObjectView<'_>,
    section_ordinal: u32,
    mismatch: PortableEmissionError,
) -> Result<RegistryCheckedRowView<'_>, PortableEmissionError> {
    view.section(section_ordinal)
        .and_then(|section| section.table(0))
        .and_then(|table| table.row(0))
        .ok_or(mismatch)
}

fn verify_artifact_diff_compatibility(
    base: RegistryCheckedObjectView<'_>,
    target: RegistryCheckedObjectView<'_>,
    base_index: &ArtifactIndex<'_>,
    target_index: &ArtifactIndex<'_>,
) -> Result<(), PortableEmissionError> {
    let base_contract_versions =
        singleton_row(base, 0, PortableEmissionError::DiffBaseSemanticMismatch)?;
    let target_contract_versions =
        singleton_row(target, 0, PortableEmissionError::InternalBindingMismatch)?;
    let base_execution_contract =
        singleton_row(base, 5, PortableEmissionError::DiffBaseSemanticMismatch)?;
    let target_execution_contract =
        singleton_row(target, 5, PortableEmissionError::InternalBindingMismatch)?;
    if base_contract_versions.bytes() != target_contract_versions.bytes()
        || base_execution_contract.bytes() != target_execution_contract.bytes()
    {
        return Err(PortableEmissionError::UnsupportedSemanticContractTransition);
    }
    for (stable_id, base_identity) in &base_index.identities {
        if let Some(target_identity) = target_index.identities.get(stable_id)
            && (base_identity.entity_kind != target_identity.entity_kind
                || base_identity.canonical_fields != target_identity.canonical_fields)
        {
            return Err(PortableEmissionError::CrossRevisionStableIdCollision);
        }
    }
    Ok(())
}

fn verify_target_relation_projection(
    output: &CompilationOutput,
    target: RegistryCheckedObjectView<'_>,
) -> Result<(), PortableEmissionError> {
    let target_index =
        ArtifactIndex::build(target, PortableEmissionError::InternalBindingMismatch)?;
    if artifact_relation_tuples(
        &target_index,
        PortableEmissionError::InternalBindingMismatch,
    )? != canonical_relation_tuples(output.lir().unit())
    {
        return Err(PortableEmissionError::InternalBindingMismatch);
    }
    Ok(())
}

fn entity_modify_tags(entity_kind: EntityKind) -> &'static [u16] {
    match entity_kind {
        EntityKind::RoadCorridor => &[3],
        EntityKind::RoadSection => &[4],
        EntityKind::AuthoringLane
        | EntityKind::Junction
        | EntityKind::Movement
        | EntityKind::ManeuverPath
        | EntityKind::WaitingZone
        | EntityKind::SignalGroup
        | EntityKind::SignalController
        | EntityKind::SignalPhase
        | EntityKind::ParkingArea
        | EntityKind::LaneGroup
        | EntityKind::AccessRule
        | EntityKind::StaticRoute
        | EntityKind::CanonicalFrame => &[],
        EntityKind::LaneEdge => &[3, 4],
        EntityKind::ManeuverGate => &[4],
        EntityKind::StopLine => &[3],
        EntityKind::ParkingSpace => &[5, 7, 8, 9, 10, 11],
        EntityKind::FacilityBand | EntityKind::ParticipantClass => &[4],
        EntityKind::VehicleProfile => &[4, 5, 6, 7, 8, 9, 10],
    }
}

fn static_rule_modify_tags(entity_kind: EntityKind) -> &'static [u16] {
    match entity_kind {
        EntityKind::ManeuverGate => &[6],
        EntityKind::WaitingZone => &[4, 5, 6],
        EntityKind::SignalController => &[3, 4],
        EntityKind::SignalPhase => &[4, 5],
        EntityKind::AccessRule => &[5, 7, 8],
        EntityKind::RoadCorridor
        | EntityKind::RoadSection
        | EntityKind::AuthoringLane
        | EntityKind::LaneEdge
        | EntityKind::Junction
        | EntityKind::Movement
        | EntityKind::ManeuverPath
        | EntityKind::StopLine
        | EntityKind::SignalGroup
        | EntityKind::ParkingArea
        | EntityKind::ParkingSpace
        | EntityKind::LaneGroup
        | EntityKind::FacilityBand
        | EntityKind::ParticipantClass
        | EntityKind::VehicleProfile
        | EntityKind::StaticRoute
        | EntityKind::CanonicalFrame => &[],
    }
}

fn stable_ref_value(
    index: &ArtifactIndex<'_>,
    entity_kind: EntityKind,
    typed_ordinal: u32,
    mismatch: PortableEmissionError,
) -> Result<Box<[u8]>, PortableEmissionError> {
    let mut value = Vec::with_capacity(18);
    value.extend_from_slice(&entity_kind.code().to_le_bytes());
    value.extend_from_slice(&index.stable_id(entity_kind, typed_ordinal, mismatch)?);
    Ok(value.into_boxed_slice())
}

fn semantic_field_value(
    index: &ArtifactIndex<'_>,
    entity_kind: EntityKind,
    entity: RegistryCheckedRowView<'_>,
    tag: u16,
    mismatch: PortableEmissionError,
) -> Result<Option<Box<[u8]>>, PortableEmissionError> {
    let Some(field) = entity.field_by_tag(tag) else {
        return Ok(None);
    };
    let stable_ref_kind = match (entity_kind, tag) {
        (EntityKind::RoadCorridor, 3) => Some(EntityKind::RoadSection),
        (EntityKind::StopLine, 3) => Some(EntityKind::LaneEdge),
        (EntityKind::WaitingZone, 4 | 5) => Some(EntityKind::ManeuverGate),
        _ => None,
    };
    if let Some(referenced_kind) = stable_ref_kind {
        return Ok(Some(stable_ref_value(
            index,
            referenced_kind,
            checked_u32_with(entity, tag, mismatch)?,
            mismatch,
        )?));
    }
    if (entity_kind, tag) == (EntityKind::SignalPhase, 5) {
        let states = checked_record_vector_with(entity, tag, mismatch)?;
        let capacity = usize::try_from(states.len())
            .map_err(|_| PortableEmissionError::ArithmeticOverflow)?
            .checked_mul(19)
            .and_then(|value| value.checked_add(4))
            .ok_or(PortableEmissionError::ArithmeticOverflow)?;
        let mut value = Vec::with_capacity(capacity);
        value.extend_from_slice(&states.len().to_le_bytes());
        for ordinal in 0..states.len() {
            let state = states.row(ordinal).ok_or(mismatch)?;
            value.extend_from_slice(&stable_ref_value(
                index,
                EntityKind::SignalGroup,
                checked_u32_with(state, 1, mismatch)?,
                mismatch,
            )?);
            value.push(checked_u8_with(state, 2, mismatch)?);
        }
        return Ok(Some(value.into_boxed_slice()));
    }
    Ok(Some(field.value_bytes().to_vec().into_boxed_slice()))
}

#[derive(Debug, Eq, PartialEq)]
struct FieldChangeProjection {
    entity_kind: EntityKind,
    stable_id: [u8; 16],
    field_tag: u16,
    before: Option<Box<[u8]>>,
    after: Option<Box<[u8]>>,
}

fn retained_field_changes(
    base: &ArtifactIndex<'_>,
    target: &ArtifactIndex<'_>,
    tags: fn(EntityKind) -> &'static [u16],
) -> Result<Vec<FieldChangeProjection>, PortableEmissionError> {
    let mut changes = Vec::new();
    for ((entity_kind, stable_id), base_entity) in &base.entities {
        let Some(target_entity) = target.entities.get(&(*entity_kind, *stable_id)) else {
            continue;
        };
        for tag in tags(*entity_kind) {
            let before = semantic_field_value(
                base,
                *entity_kind,
                base_entity.row,
                *tag,
                PortableEmissionError::DiffBaseSemanticMismatch,
            )?;
            let after = semantic_field_value(
                target,
                *entity_kind,
                target_entity.row,
                *tag,
                PortableEmissionError::InternalBindingMismatch,
            )?;
            if before != after {
                changes.push(FieldChangeProjection {
                    entity_kind: *entity_kind,
                    stable_id: *stable_id,
                    field_tag: *tag,
                    before,
                    after,
                });
            }
        }
    }
    changes.sort_unstable_by_key(|change| (change.entity_kind, change.stable_id, change.field_tag));
    Ok(changes)
}

fn artifact_entity_changes(
    base: &ArtifactIndex<'_>,
    target: &ArtifactIndex<'_>,
) -> Result<Vec<OwnedRow>, PortableEmissionError> {
    let mut changes = Vec::<(u8, EntityKind, [u8; 16], u16, OwnedRow)>::new();
    for ((entity_kind, stable_id), entity) in &target.entities {
        if !base.entities.contains_key(&(*entity_kind, *stable_id)) {
            changes.push((
                0,
                *entity_kind,
                *stable_id,
                0,
                row([
                    field(1, OwnedValue::U8(0)),
                    field(2, OwnedValue::U16(entity_kind.code())),
                    field(4, OwnedValue::StableId128(*stable_id)),
                    field(
                        10,
                        OwnedValue::Bytes(entity.row.bytes().to_vec().into_boxed_slice()),
                    ),
                ]),
            ));
        }
    }
    for ((entity_kind, stable_id), entity) in &base.entities {
        if !target.entities.contains_key(&(*entity_kind, *stable_id)) {
            changes.push((
                1,
                *entity_kind,
                *stable_id,
                0,
                row([
                    field(1, OwnedValue::U8(1)),
                    field(2, OwnedValue::U16(entity_kind.code())),
                    field(4, OwnedValue::StableId128(*stable_id)),
                    field(
                        9,
                        OwnedValue::Bytes(entity.row.bytes().to_vec().into_boxed_slice()),
                    ),
                ]),
            ));
        }
    }
    for change in retained_field_changes(base, target, entity_modify_tags)? {
        let mut fields = vec![
            field(1, OwnedValue::U8(2)),
            field(2, OwnedValue::U16(change.entity_kind.code())),
            field(4, OwnedValue::StableId128(change.stable_id)),
            field(6, OwnedValue::U16(change.field_tag)),
        ];
        if let Some(before) = change.before {
            fields.push(field(9, OwnedValue::Bytes(before)));
        }
        if let Some(after) = change.after {
            fields.push(field(10, OwnedValue::Bytes(after)));
        }
        changes.push((
            2,
            change.entity_kind,
            change.stable_id,
            change.field_tag,
            row(fields),
        ));
    }
    changes.sort_unstable_by_key(|(change_kind, entity_kind, stable_id, field_tag, _)| {
        (*change_kind, *entity_kind, *stable_id, *field_tag)
    });
    Ok(changes.into_iter().map(|(_, _, _, _, row)| row).collect())
}

fn artifact_static_rule_changes(
    base: &ArtifactIndex<'_>,
    target: &ArtifactIndex<'_>,
) -> Result<Vec<OwnedRow>, PortableEmissionError> {
    retained_field_changes(base, target, static_rule_modify_tags)?
        .into_iter()
        .map(|change| {
            let mut fields = vec![
                field(1, OwnedValue::U8(0)),
                field(2, OwnedValue::U16(change.entity_kind.code())),
                field(4, OwnedValue::StableId128(change.stable_id)),
                field(6, OwnedValue::U16(change.field_tag)),
            ];
            if let Some(before) = change.before {
                fields.push(field(9, OwnedValue::Bytes(before)));
            }
            if let Some(after) = change.after {
                fields.push(field(10, OwnedValue::Bytes(after)));
            }
            Ok(row(fields))
        })
        .collect()
}

#[allow(
    clippy::too_many_arguments,
    reason = "the closed A.5 tuple is clearer when every semantic coordinate is explicit"
)]
fn push_artifact_relation(
    relations: &mut Vec<RelationTuple>,
    index: &ArtifactIndex<'_>,
    owner_entity_kind: EntityKind,
    owner_stable_id: [u8; 16],
    role: u8,
    local_index: u32,
    subject_entity_kind: EntityKind,
    subject_ordinal: u32,
    mismatch: PortableEmissionError,
) -> Result<(), PortableEmissionError> {
    relations.push(RelationTuple {
        owner_entity_kind,
        owner_stable_id,
        role,
        local_index,
        subject_entity_kind,
        subject_stable_id: index.stable_id(subject_entity_kind, subject_ordinal, mismatch)?,
    });
    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "the closed A.5 vector projection keeps owner, field, role, and subject kind explicit"
)]
fn push_vector_relations(
    relations: &mut Vec<RelationTuple>,
    index: &ArtifactIndex<'_>,
    owner_entity_kind: EntityKind,
    owner_stable_id: [u8; 16],
    owner: RegistryCheckedRowView<'_>,
    field_tag: u16,
    role: u8,
    subject_entity_kind: EntityKind,
    mismatch: PortableEmissionError,
) -> Result<(), PortableEmissionError> {
    let values = checked_ordinal_vector_with(owner, field_tag, mismatch)?;
    for local_index in 0..values.len() {
        push_artifact_relation(
            relations,
            index,
            owner_entity_kind,
            owner_stable_id,
            role,
            local_index,
            subject_entity_kind,
            values.get(local_index).ok_or(mismatch)?,
            mismatch,
        )?;
    }
    Ok(())
}

fn artifact_relation_tuples(
    index: &ArtifactIndex<'_>,
    mismatch: PortableEmissionError,
) -> Result<Vec<RelationTuple>, PortableEmissionError> {
    let mut relations = Vec::new();
    for ((owner_kind, owner_stable_id), owner) in &index.entities {
        match owner_kind {
            EntityKind::RoadCorridor => {
                let elements = checked_record_vector_with(owner.row, 4, mismatch)?;
                for local_index in 0..elements.len() {
                    let element = elements.row(local_index).ok_or(mismatch)?;
                    let subject_kind = match checked_u8_with(element, 1, mismatch)? {
                        0 => EntityKind::RoadSection,
                        1 => EntityKind::FacilityBand,
                        _ => return Err(mismatch),
                    };
                    push_artifact_relation(
                        &mut relations,
                        index,
                        *owner_kind,
                        *owner_stable_id,
                        2,
                        local_index,
                        subject_kind,
                        checked_u32_with(element, 2, mismatch)?,
                        mismatch,
                    )?;
                }
            }
            EntityKind::RoadSection => push_vector_relations(
                &mut relations,
                index,
                *owner_kind,
                *owner_stable_id,
                owner.row,
                5,
                3,
                EntityKind::AuthoringLane,
                mismatch,
            )?,
            EntityKind::AuthoringLane => push_vector_relations(
                &mut relations,
                index,
                *owner_kind,
                *owner_stable_id,
                owner.row,
                4,
                4,
                EntityKind::LaneEdge,
                mismatch,
            )?,
            EntityKind::LaneEdge => push_vector_relations(
                &mut relations,
                index,
                *owner_kind,
                *owner_stable_id,
                owner.row,
                5,
                1,
                EntityKind::LaneEdge,
                mismatch,
            )?,
            EntityKind::Junction => push_vector_relations(
                &mut relations,
                index,
                *owner_kind,
                *owner_stable_id,
                owner.row,
                3,
                6,
                EntityKind::Movement,
                mismatch,
            )?,
            EntityKind::Movement => push_vector_relations(
                &mut relations,
                index,
                *owner_kind,
                *owner_stable_id,
                owner.row,
                6,
                7,
                EntityKind::ManeuverPath,
                mismatch,
            )?,
            EntityKind::ManeuverPath => {
                for (field_tag, role, subject_kind) in [
                    (4, 8, EntityKind::LaneEdge),
                    (5, 10, EntityKind::ManeuverGate),
                    (6, 11, EntityKind::WaitingZone),
                ] {
                    push_vector_relations(
                        &mut relations,
                        index,
                        *owner_kind,
                        *owner_stable_id,
                        owner.row,
                        field_tag,
                        role,
                        subject_kind,
                        mismatch,
                    )?;
                }
            }
            EntityKind::ManeuverGate => {
                if let Some(signal_group) = owner.row.field_by_tag(7) {
                    push_artifact_relation(
                        &mut relations,
                        index,
                        *owner_kind,
                        *owner_stable_id,
                        20,
                        0,
                        EntityKind::SignalGroup,
                        match signal_group.value()? {
                            RegistryCheckedFieldValue::U32(value) => value,
                            _ => return Err(mismatch),
                        },
                        mismatch,
                    )?;
                }
            }
            EntityKind::StopLine => push_vector_relations(
                &mut relations,
                index,
                *owner_kind,
                *owner_stable_id,
                owner.row,
                4,
                12,
                EntityKind::ManeuverGate,
                mismatch,
            )?,
            EntityKind::SignalController => {
                for (field_tag, role, subject_kind) in [
                    (5, 17, EntityKind::SignalGroup),
                    (6, 18, EntityKind::SignalPhase),
                ] {
                    push_vector_relations(
                        &mut relations,
                        index,
                        *owner_kind,
                        *owner_stable_id,
                        owner.row,
                        field_tag,
                        role,
                        subject_kind,
                        mismatch,
                    )?;
                }
            }
            EntityKind::ParkingSpace => {
                if let Some(parking_area) = owner.row.field_by_tag(3) {
                    let ordinal = match parking_area.value()? {
                        RegistryCheckedFieldValue::U32(value) => value,
                        _ => return Err(mismatch),
                    };
                    push_artifact_relation(
                        &mut relations,
                        index,
                        *owner_kind,
                        *owner_stable_id,
                        21,
                        0,
                        EntityKind::ParkingArea,
                        ordinal,
                        mismatch,
                    )?;
                }
                for (field_tag, role) in [(4, 22), (6, 23)] {
                    push_artifact_relation(
                        &mut relations,
                        index,
                        *owner_kind,
                        *owner_stable_id,
                        role,
                        0,
                        EntityKind::LaneEdge,
                        checked_u32_with(owner.row, field_tag, mismatch)?,
                        mismatch,
                    )?;
                }
            }
            EntityKind::LaneGroup => push_vector_relations(
                &mut relations,
                index,
                *owner_kind,
                *owner_stable_id,
                owner.row,
                4,
                5,
                EntityKind::AuthoringLane,
                mismatch,
            )?,
            EntityKind::ParticipantClass => {
                if let Some(parent) = owner.row.field_by_tag(3) {
                    let ordinal = match parent.value()? {
                        RegistryCheckedFieldValue::U32(value) => value,
                        _ => return Err(mismatch),
                    };
                    push_artifact_relation(
                        &mut relations,
                        index,
                        *owner_kind,
                        *owner_stable_id,
                        24,
                        0,
                        EntityKind::ParticipantClass,
                        ordinal,
                        mismatch,
                    )?;
                }
            }
            EntityKind::AccessRule => {
                let subject_kind = match checked_u8_with(owner.row, 3, mismatch)? {
                    0 => EntityKind::LaneEdge,
                    1 => EntityKind::LaneGroup,
                    2 => EntityKind::RoadSection,
                    3 => EntityKind::ManeuverPath,
                    _ => return Err(mismatch),
                };
                push_artifact_relation(
                    &mut relations,
                    index,
                    *owner_kind,
                    *owner_stable_id,
                    25,
                    0,
                    subject_kind,
                    checked_u32_with(owner.row, 4, mismatch)?,
                    mismatch,
                )?;
                push_vector_relations(
                    &mut relations,
                    index,
                    *owner_kind,
                    *owner_stable_id,
                    owner.row,
                    6,
                    26,
                    EntityKind::ParticipantClass,
                    mismatch,
                )?;
            }
            EntityKind::VehicleProfile => push_artifact_relation(
                &mut relations,
                index,
                *owner_kind,
                *owner_stable_id,
                27,
                0,
                EntityKind::ParticipantClass,
                checked_u32_with(owner.row, 3, mismatch)?,
                mismatch,
            )?,
            EntityKind::StaticRoute => push_vector_relations(
                &mut relations,
                index,
                *owner_kind,
                *owner_stable_id,
                owner.row,
                3,
                13,
                EntityKind::LaneEdge,
                mismatch,
            )?,
            EntityKind::WaitingZone
            | EntityKind::SignalGroup
            | EntityKind::SignalPhase
            | EntityKind::ParkingArea
            | EntityKind::FacilityBand
            | EntityKind::CanonicalFrame => {}
        }
    }

    let relation_section = index.view.section(3).ok_or(mismatch)?;
    let internal_edges = relation_section.table(0).ok_or(mismatch)?;
    let mut next_internal_index = BTreeMap::<[u8; 16], u32>::new();
    for ordinal in 0..internal_edges.row_count() {
        let relation = internal_edges.row(ordinal).ok_or(mismatch)?;
        let owner_stable_id = index.stable_id(
            EntityKind::Junction,
            checked_u32_with(relation, 2, mismatch)?,
            mismatch,
        )?;
        let local_index = next_internal_index.entry(owner_stable_id).or_default();
        push_artifact_relation(
            &mut relations,
            index,
            EntityKind::Junction,
            owner_stable_id,
            9,
            *local_index,
            EntityKind::LaneEdge,
            checked_u32_with(relation, 1, mismatch)?,
            mismatch,
        )?;
        *local_index = local_index
            .checked_add(1)
            .ok_or(PortableEmissionError::ArithmeticOverflow)?;
    }
    for (table_ordinal, role, subject_kind) in [
        (1, 14, EntityKind::ManeuverPath),
        (2, 15, EntityKind::ManeuverGate),
        (3, 16, EntityKind::WaitingZone),
    ] {
        let occurrences = relation_section.table(table_ordinal).ok_or(mismatch)?;
        for ordinal in 0..occurrences.row_count() {
            let occurrence = occurrences.row(ordinal).ok_or(mismatch)?;
            let owner_stable_id = index.stable_id(
                EntityKind::StaticRoute,
                checked_u32_with(occurrence, 1, mismatch)?,
                mismatch,
            )?;
            push_artifact_relation(
                &mut relations,
                index,
                EntityKind::StaticRoute,
                owner_stable_id,
                role,
                checked_u32_with(occurrence, 2, mismatch)?,
                subject_kind,
                checked_u32_with(occurrence, 3, mismatch)?,
                mismatch,
            )?;
        }
    }
    relations.sort_unstable();
    Ok(relations)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RelationPairing {
    Set,
    Scalar,
    DomainOccurrence,
}

fn relation_pairing(role: u8) -> Option<RelationPairing> {
    match role {
        1 | 6 | 7 | 9 | 12 | 17 | 26 => Some(RelationPairing::Set),
        20..=25 | 27 => Some(RelationPairing::Scalar),
        2..=5 | 8 | 10 | 11 | 13..=16 | 18 => Some(RelationPairing::DomainOccurrence),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RelationChangeProjection {
    change_kind: u8,
    owner_entity_kind: EntityKind,
    owner_stable_id: [u8; 16],
    role: u8,
    subject_stable_id: Option<[u8; 16]>,
    before_local_index: Option<u32>,
    after_local_index: Option<u32>,
    before_target: Option<[u8; 16]>,
    after_target: Option<[u8; 16]>,
}

fn relation_add(tuple: RelationTuple) -> RelationChangeProjection {
    RelationChangeProjection {
        change_kind: 0,
        owner_entity_kind: tuple.owner_entity_kind,
        owner_stable_id: tuple.owner_stable_id,
        role: tuple.role,
        subject_stable_id: Some(tuple.subject_stable_id),
        before_local_index: None,
        after_local_index: Some(tuple.local_index),
        before_target: None,
        after_target: None,
    }
}

fn relation_remove(tuple: RelationTuple) -> RelationChangeProjection {
    RelationChangeProjection {
        change_kind: 1,
        owner_entity_kind: tuple.owner_entity_kind,
        owner_stable_id: tuple.owner_stable_id,
        role: tuple.role,
        subject_stable_id: Some(tuple.subject_stable_id),
        before_local_index: Some(tuple.local_index),
        after_local_index: None,
        before_target: None,
        after_target: None,
    }
}

fn relation_move(before: RelationTuple, after: RelationTuple) -> RelationChangeProjection {
    RelationChangeProjection {
        change_kind: 2,
        owner_entity_kind: before.owner_entity_kind,
        owner_stable_id: before.owner_stable_id,
        role: before.role,
        subject_stable_id: Some(before.subject_stable_id),
        before_local_index: Some(before.local_index),
        after_local_index: Some(after.local_index),
        before_target: None,
        after_target: None,
    }
}

fn relation_reconnect(before: RelationTuple, after: RelationTuple) -> RelationChangeProjection {
    RelationChangeProjection {
        change_kind: 3,
        owner_entity_kind: before.owner_entity_kind,
        owner_stable_id: before.owner_stable_id,
        role: before.role,
        subject_stable_id: None,
        before_local_index: Some(before.local_index),
        after_local_index: Some(after.local_index),
        before_target: Some(before.subject_stable_id),
        after_target: Some(after.subject_stable_id),
    }
}

fn compare_relation_changes(
    left: &RelationChangeProjection,
    right: &RelationChangeProjection,
) -> std::cmp::Ordering {
    left.change_kind
        .cmp(&right.change_kind)
        .then_with(|| left.owner_entity_kind.cmp(&right.owner_entity_kind))
        .then_with(|| left.owner_stable_id.cmp(&right.owner_stable_id))
        .then_with(|| left.role.cmp(&right.role))
        .then_with(|| match left.change_kind {
            0 => left
                .after_local_index
                .cmp(&right.after_local_index)
                .then_with(|| left.subject_stable_id.cmp(&right.subject_stable_id)),
            1 => left
                .before_local_index
                .cmp(&right.before_local_index)
                .then_with(|| left.subject_stable_id.cmp(&right.subject_stable_id)),
            2 => left
                .before_local_index
                .cmp(&right.before_local_index)
                .then_with(|| left.after_local_index.cmp(&right.after_local_index))
                .then_with(|| left.subject_stable_id.cmp(&right.subject_stable_id)),
            3 => left
                .before_local_index
                .cmp(&right.before_local_index)
                .then_with(|| left.after_local_index.cmp(&right.after_local_index))
                .then_with(|| left.before_target.cmp(&right.before_target))
                .then_with(|| left.after_target.cmp(&right.after_target)),
            _ => std::cmp::Ordering::Equal,
        })
}

fn group_relations(
    relations: Vec<RelationTuple>,
    mismatch: PortableEmissionError,
) -> Result<RelationGroups, PortableEmissionError> {
    let mut groups = BTreeMap::new();
    for relation in relations {
        groups
            .entry((
                relation.owner_entity_kind,
                relation.owner_stable_id,
                relation.role,
            ))
            .or_insert_with(Vec::new)
            .push(relation);
    }
    for relations in groups.values_mut() {
        relations.sort_unstable_by_key(|relation| relation.local_index);
        for (expected, relation) in relations.iter().enumerate() {
            if relation.local_index
                != u32::try_from(expected).map_err(|_| PortableEmissionError::ArithmeticOverflow)?
            {
                return Err(mismatch);
            }
        }
    }
    Ok(groups)
}

fn pair_set_relations(
    base: &[RelationTuple],
    target: &[RelationTuple],
    changes: &mut Vec<RelationChangeProjection>,
) -> Result<(), PortableEmissionError> {
    let mut base_members = BTreeMap::new();
    for relation in base {
        if base_members
            .insert(
                (relation.subject_entity_kind, relation.subject_stable_id),
                *relation,
            )
            .is_some()
        {
            return Err(PortableEmissionError::DiffBaseSemanticMismatch);
        }
    }
    let mut target_members = BTreeMap::new();
    for relation in target {
        if target_members
            .insert(
                (relation.subject_entity_kind, relation.subject_stable_id),
                *relation,
            )
            .is_some()
        {
            return Err(PortableEmissionError::InternalBindingMismatch);
        }
    }
    changes.extend(
        base_members
            .iter()
            .filter(|(subject, _)| !target_members.contains_key(subject))
            .map(|(_, relation)| relation_remove(*relation)),
    );
    changes.extend(
        target_members
            .iter()
            .filter(|(subject, _)| !base_members.contains_key(subject))
            .map(|(_, relation)| relation_add(*relation)),
    );
    Ok(())
}

fn pair_scalar_relations(
    base: &[RelationTuple],
    target: &[RelationTuple],
    changes: &mut Vec<RelationChangeProjection>,
) -> Result<(), PortableEmissionError> {
    if base.len() > 1 {
        return Err(PortableEmissionError::DiffBaseSemanticMismatch);
    }
    if target.len() > 1 {
        return Err(PortableEmissionError::InternalBindingMismatch);
    }
    match (base.first().copied(), target.first().copied()) {
        (None, None) => {}
        (None, Some(after)) => changes.push(relation_add(after)),
        (Some(before), None) => changes.push(relation_remove(before)),
        (Some(before), Some(after))
            if (before.subject_entity_kind, before.subject_stable_id)
                != (after.subject_entity_kind, after.subject_stable_id) =>
        {
            changes.push(relation_reconnect(before, after));
        }
        (Some(_), Some(_)) => {}
    }
    Ok(())
}

fn pair_domain_relations(
    base: &[RelationTuple],
    target: &[RelationTuple],
    changes: &mut Vec<RelationChangeProjection>,
) {
    let mut base_occurrences = BTreeMap::<(EntityKind, [u8; 16]), Vec<RelationTuple>>::new();
    let mut target_occurrences = BTreeMap::<(EntityKind, [u8; 16]), Vec<RelationTuple>>::new();
    for relation in base {
        base_occurrences
            .entry((relation.subject_entity_kind, relation.subject_stable_id))
            .or_default()
            .push(*relation);
    }
    for relation in target {
        target_occurrences
            .entry((relation.subject_entity_kind, relation.subject_stable_id))
            .or_default()
            .push(*relation);
    }
    for occurrences in base_occurrences.values_mut() {
        occurrences.sort_unstable_by_key(|relation| relation.local_index);
    }
    for occurrences in target_occurrences.values_mut() {
        occurrences.sort_unstable_by_key(|relation| relation.local_index);
    }
    let mut subjects: Vec<_> = base_occurrences
        .keys()
        .chain(target_occurrences.keys())
        .copied()
        .collect();
    subjects.sort_unstable();
    subjects.dedup();
    for subject in &subjects {
        let before = base_occurrences.get(subject).map_or(&[][..], Vec::as_slice);
        let after = target_occurrences
            .get(subject)
            .map_or(&[][..], Vec::as_slice);
        let paired_count = before.len().min(after.len());
        for rank in 0..paired_count {
            if before[rank].local_index != after[rank].local_index {
                changes.push(relation_move(before[rank], after[rank]));
            }
        }
        changes.extend(before[paired_count..].iter().copied().map(relation_remove));
        changes.extend(after[paired_count..].iter().copied().map(relation_add));
    }
}

fn relation_change_row(change: RelationChangeProjection) -> OwnedRow {
    let mut fields = vec![
        field(1, OwnedValue::U8(change.change_kind)),
        field(2, OwnedValue::U16(change.owner_entity_kind.code())),
        field(3, OwnedValue::StableId128(change.owner_stable_id)),
    ];
    if let Some(subject) = change.subject_stable_id {
        fields.push(field(4, OwnedValue::StableId128(subject)));
    }
    fields.push(field(5, OwnedValue::U8(change.role)));
    if let Some(index) = change.before_local_index {
        fields.push(field(7, OwnedValue::U32(index)));
    }
    if let Some(index) = change.after_local_index {
        fields.push(field(8, OwnedValue::U32(index)));
    }
    if let Some(target) = change.before_target {
        fields.push(field(9, OwnedValue::StableId128(target)));
    }
    if let Some(target) = change.after_target {
        fields.push(field(10, OwnedValue::StableId128(target)));
    }
    row(fields)
}

fn artifact_relation_changes(
    base: &ArtifactIndex<'_>,
    target: &ArtifactIndex<'_>,
) -> Result<Vec<OwnedRow>, PortableEmissionError> {
    let mut base_groups = group_relations(
        artifact_relation_tuples(base, PortableEmissionError::DiffBaseSemanticMismatch)?,
        PortableEmissionError::DiffBaseSemanticMismatch,
    )?;
    let mut target_groups = group_relations(
        artifact_relation_tuples(target, PortableEmissionError::InternalBindingMismatch)?,
        PortableEmissionError::InternalBindingMismatch,
    )?;
    let mut group_keys: Vec<_> = base_groups
        .keys()
        .chain(target_groups.keys())
        .copied()
        .collect();
    group_keys.sort_unstable();
    group_keys.dedup();
    let mut changes = Vec::new();
    for key in group_keys {
        let base_relations = base_groups.remove(&key).unwrap_or_default();
        let target_relations = target_groups.remove(&key).unwrap_or_default();
        match relation_pairing(key.2).ok_or(PortableEmissionError::InternalBindingMismatch)? {
            RelationPairing::Set => {
                pair_set_relations(&base_relations, &target_relations, &mut changes)?;
            }
            RelationPairing::Scalar => {
                pair_scalar_relations(&base_relations, &target_relations, &mut changes)?;
            }
            RelationPairing::DomainOccurrence => {
                pair_domain_relations(&base_relations, &target_relations, &mut changes);
            }
        }
    }
    changes.sort_unstable_by(compare_relation_changes);
    Ok(changes.into_iter().map(relation_change_row).collect())
}

fn artifact_geometry_values(
    index: &ArtifactIndex<'_>,
    mismatch: PortableEmissionError,
) -> Result<GeometryValues, PortableEmissionError> {
    let spatial = index.view.section(4).ok_or(mismatch)?;
    let mut geometries = BTreeMap::new();
    for (table_ordinal, subject_kind, projected_tags) in [
        (1, EntityKind::LaneEdge, &[3_u16, 4, 5, 6][..]),
        (2, EntityKind::FacilityBand, &[3_u16, 4][..]),
    ] {
        let table = spatial.table(table_ordinal).ok_or(mismatch)?;
        for ordinal in 0..table.row_count() {
            let geometry = table.row(ordinal).ok_or(mismatch)?;
            let subject_stable_id = index.stable_id(
                subject_kind,
                checked_u32_with(geometry, 1, mismatch)?,
                mismatch,
            )?;
            let frame_stable_id = index.stable_id(
                EntityKind::CanonicalFrame,
                checked_u32_with(geometry, 2, mismatch)?,
                mismatch,
            )?;
            if geometries
                .insert(
                    (subject_kind, subject_stable_id),
                    canonical_geometry_value(geometry, frame_stable_id, projected_tags)?,
                )
                .is_some()
            {
                return Err(mismatch);
            }
        }
    }
    Ok(geometries)
}

fn artifact_geometry_changes(
    base: &ArtifactIndex<'_>,
    target: &ArtifactIndex<'_>,
) -> Result<Vec<OwnedRow>, PortableEmissionError> {
    let base_values =
        artifact_geometry_values(base, PortableEmissionError::DiffBaseSemanticMismatch)?;
    let target_values =
        artifact_geometry_values(target, PortableEmissionError::InternalBindingMismatch)?;
    let mut changes = Vec::<(u8, EntityKind, [u8; 16], OwnedRow)>::new();
    for ((entity_kind, stable_id), after) in &target_values {
        match base_values.get(&(*entity_kind, *stable_id)) {
            None => changes.push((
                0,
                *entity_kind,
                *stable_id,
                row([
                    field(1, OwnedValue::U8(0)),
                    field(2, OwnedValue::U16(entity_kind.code())),
                    field(4, OwnedValue::StableId128(*stable_id)),
                    field(10, OwnedValue::Bytes(after.clone())),
                ]),
            )),
            Some(before) if before != after => changes.push((
                2,
                *entity_kind,
                *stable_id,
                row([
                    field(1, OwnedValue::U8(2)),
                    field(2, OwnedValue::U16(entity_kind.code())),
                    field(4, OwnedValue::StableId128(*stable_id)),
                    field(9, OwnedValue::Bytes(before.clone())),
                    field(10, OwnedValue::Bytes(after.clone())),
                ]),
            )),
            Some(_) => {}
        }
    }
    for ((entity_kind, stable_id), before) in &base_values {
        if !target_values.contains_key(&(*entity_kind, *stable_id)) {
            changes.push((
                1,
                *entity_kind,
                *stable_id,
                row([
                    field(1, OwnedValue::U8(1)),
                    field(2, OwnedValue::U16(entity_kind.code())),
                    field(4, OwnedValue::StableId128(*stable_id)),
                    field(9, OwnedValue::Bytes(before.clone())),
                ]),
            ));
        }
    }
    changes.sort_unstable_by_key(|(change_kind, entity_kind, stable_id, _)| {
        (*change_kind, *entity_kind, *stable_id)
    });
    Ok(changes.into_iter().map(|(_, _, _, row)| row).collect())
}

fn artifact_spatial_configuration_changes(
    base: &ArtifactIndex<'_>,
    target: &ArtifactIndex<'_>,
) -> Result<Vec<OwnedRow>, PortableEmissionError> {
    let before = singleton_row(
        base.view,
        4,
        PortableEmissionError::DiffBaseSemanticMismatch,
    )?;
    let after = singleton_row(
        target.view,
        4,
        PortableEmissionError::InternalBindingMismatch,
    )?;
    if before.bytes() == after.bytes() {
        return Ok(Vec::new());
    }
    Ok(vec![row([
        field(1, OwnedValue::U8(1)),
        field(
            2,
            OwnedValue::Bytes(before.bytes().to_vec().into_boxed_slice()),
        ),
        field(
            3,
            OwnedValue::Bytes(after.bytes().to_vec().into_boxed_slice()),
        ),
    ])])
}

fn build_artifact_lfsd(
    base: ValueCheckedObjectView<'_>,
    target_network_revision: [u8; 32],
    target_artifact: &PortableObjectCandidate,
    limits: FormatLimits,
) -> Result<OwnedObject, PortableEmissionError> {
    if base.kind() != PortableObjectKind::CanonicalArtifact {
        return Err(PortableEmissionError::InvalidDiffBaseKind);
    }
    let base_view = base.registry_view();
    let target_view = preflight_object_values_v1(
        target_artifact.bytes(),
        PortableObjectKind::CanonicalArtifact,
        limits,
    )?
    .registry_view();
    let base_index =
        ArtifactIndex::build(base_view, PortableEmissionError::DiffBaseSemanticMismatch)?;
    let target_index =
        ArtifactIndex::build(target_view, PortableEmissionError::InternalBindingMismatch)?;
    verify_artifact_diff_compatibility(base_view, target_view, &base_index, &target_index)?;

    let base_network_revision = network_revision(base.bytes(), limits)?;
    let base_digest = sha256(base.bytes());
    let base_length =
        u64::try_from(base.bytes().len()).map_err(|_| PortableEmissionError::ArithmeticOverflow)?;
    let entity_changes = artifact_entity_changes(&base_index, &target_index)?;
    let relation_changes = artifact_relation_changes(&base_index, &target_index)?;
    let geometry_changes = artifact_geometry_changes(&base_index, &target_index)?;
    let static_rule_changes = artifact_static_rule_changes(&base_index, &target_index)?;
    let spatial_changes = artifact_spatial_configuration_changes(&base_index, &target_index)?;

    Ok(OwnedObject {
        kind: PortableObjectKind::SemanticDiff,
        sections: vec![
            section(
                1,
                [table(
                    1,
                    [row([
                        field(1, OwnedValue::U8(1)),
                        field(2, OwnedValue::U16(NETWORK_REVISION_DERIVATION_VERSION)),
                        field(3, OwnedValue::Sha256(base_network_revision)),
                        field(4, OwnedValue::Sha256(base_digest)),
                        field(5, OwnedValue::U64(base_length)),
                        field(6, OwnedValue::U16(NETWORK_REVISION_DERIVATION_VERSION)),
                        field(7, OwnedValue::Sha256(target_network_revision)),
                        field(8, OwnedValue::Sha256(target_artifact.digest())),
                        field(9, OwnedValue::U64(target_artifact.byte_length())),
                    ])],
                )],
            ),
            section(2, [table(1, entity_changes)]),
            section(3, [table(1, relation_changes)]),
            section(4, [table(1, geometry_changes)]),
            section(5, [table(1, static_rule_changes)]),
            section(6, [table(1, spatial_changes)]),
        ]
        .into_boxed_slice(),
    })
}

fn checked_u32(row: RegistryCheckedRowView<'_>, tag: u16) -> Result<u32, PortableEmissionError> {
    match row
        .field_by_tag(tag)
        .ok_or(PortableEmissionError::InternalBindingMismatch)?
        .value()?
    {
        RegistryCheckedFieldValue::U32(value) => Ok(value),
        _ => Err(PortableEmissionError::InternalBindingMismatch),
    }
}

fn checked_stable_id(
    row: RegistryCheckedRowView<'_>,
    tag: u16,
) -> Result<[u8; 16], PortableEmissionError> {
    match row
        .field_by_tag(tag)
        .ok_or(PortableEmissionError::InternalBindingMismatch)?
        .value()?
    {
        RegistryCheckedFieldValue::StableId128(value) => Ok(value.into_bytes()),
        _ => Err(PortableEmissionError::InternalBindingMismatch),
    }
}

fn genesis_entity_changes(
    target: RegistryCheckedObjectView<'_>,
) -> Result<Vec<OwnedRow>, PortableEmissionError> {
    let section = target
        .section(2)
        .ok_or(PortableEmissionError::InternalBindingMismatch)?;
    let mut changes = Vec::new();
    for (table_index, entity_kind) in EntityKind::ALL.into_iter().enumerate() {
        let table = section
            .table(
                u32::try_from(table_index)
                    .expect("the canonical entity registry contains only 22 tables"),
            )
            .ok_or(PortableEmissionError::InternalBindingMismatch)?;
        for row_index in 0..table.row_count() {
            let entity = table
                .row(row_index)
                .ok_or(PortableEmissionError::InternalBindingMismatch)?;
            changes.push((
                entity_kind,
                checked_stable_id(entity, 2)?,
                entity.bytes().to_vec().into_boxed_slice(),
            ));
        }
    }
    changes.sort_unstable_by_key(|(kind, stable_id, _)| (*kind, *stable_id));
    Ok(changes
        .into_iter()
        .map(|(kind, stable_id, bytes)| {
            row([
                field(1, OwnedValue::U8(0)),
                field(2, OwnedValue::U16(kind.code())),
                field(4, OwnedValue::StableId128(stable_id)),
                field(10, OwnedValue::Bytes(bytes)),
            ])
        })
        .collect())
}

fn entity_stable_id(lir: &crate::lir::LirUnit, kind: EntityKind, ordinal: u32) -> [u8; 16] {
    let index = usize::try_from(ordinal)
        .expect("supported compiler targets can index a validated entity ordinal");
    match kind {
        EntityKind::RoadCorridor => stable_id_bytes(lir.road_corridors[index].stable_id),
        EntityKind::RoadSection => stable_id_bytes(lir.road_sections[index].stable_id),
        EntityKind::AuthoringLane => stable_id_bytes(lir.authoring_lanes[index].stable_id),
        EntityKind::LaneEdge => stable_id_bytes(lir.lane_edges[index].stable_id),
        EntityKind::Junction => stable_id_bytes(lir.junctions[index].stable_id),
        EntityKind::Movement => stable_id_bytes(lir.movements[index].stable_id),
        EntityKind::ManeuverPath => stable_id_bytes(lir.maneuver_paths[index].stable_id),
        EntityKind::ManeuverGate => stable_id_bytes(lir.maneuver_gates[index].stable_id),
        EntityKind::WaitingZone => stable_id_bytes(lir.waiting_zones[index].stable_id),
        EntityKind::StopLine => stable_id_bytes(lir.stop_lines[index].stable_id),
        EntityKind::SignalGroup => stable_id_bytes(lir.signal_groups[index].stable_id),
        EntityKind::SignalController => stable_id_bytes(lir.signal_controllers[index].stable_id),
        EntityKind::SignalPhase => stable_id_bytes(lir.signal_phases[index].stable_id),
        EntityKind::ParkingArea => stable_id_bytes(lir.parking_areas[index].stable_id),
        EntityKind::ParkingSpace => stable_id_bytes(lir.parking_spaces[index].stable_id),
        EntityKind::LaneGroup => stable_id_bytes(lir.lane_groups[index].stable_id),
        EntityKind::FacilityBand => stable_id_bytes(lir.facility_bands[index].stable_id),
        EntityKind::ParticipantClass => stable_id_bytes(lir.participant_classes[index].stable_id),
        EntityKind::AccessRule => stable_id_bytes(lir.access_rules[index].stable_id),
        EntityKind::VehicleProfile => stable_id_bytes(lir.vehicle_profiles[index].stable_id),
        EntityKind::StaticRoute => stable_id_bytes(lir.static_routes[index].stable_id),
        EntityKind::CanonicalFrame => stable_id_bytes(lir.canonical_frames[index].stable_id),
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "the compiler-private A.5 tuple projection keeps both typed endpoints explicit"
)]
fn push_relation_tuple(
    relations: &mut Vec<RelationTuple>,
    lir: &crate::lir::LirUnit,
    owner_kind: EntityKind,
    owner_ordinal: u32,
    role: u8,
    local_index: u32,
    subject_kind: EntityKind,
    subject_ordinal: u32,
) {
    relations.push(RelationTuple {
        owner_entity_kind: owner_kind,
        owner_stable_id: entity_stable_id(lir, owner_kind, owner_ordinal),
        role,
        local_index,
        subject_entity_kind: subject_kind,
        subject_stable_id: entity_stable_id(lir, subject_kind, subject_ordinal),
    });
}

fn canonical_relation_tuples(lir: &crate::lir::LirUnit) -> Vec<RelationTuple> {
    let mut relations = Vec::new();
    let internal_edges: Vec<bool> = (0..lir.lane_edges.len())
        .map(|ordinal| {
            lir.junction_internal_edges
                .binary_search_by_key(
                    &u32::try_from(ordinal).expect("compile limits cap entity counts at u32"),
                    |entry| entry.edge.raw(),
                )
                .is_ok()
        })
        .collect();
    for edge in &lir.lane_edges {
        let mut local_index = 0_u32;
        for successor in &lir.lane_edge_successors[edge.successors.as_usize_range()] {
            if internal_edges[edge.ordinal.index()] || internal_edges[successor.index()] {
                continue;
            }
            push_relation_tuple(
                &mut relations,
                lir,
                EntityKind::LaneEdge,
                edge.ordinal.raw(),
                1,
                local_index,
                EntityKind::LaneEdge,
                successor.raw(),
            );
            local_index += 1;
        }
    }
    for corridor in &lir.road_corridors {
        for (local_index, element) in lir.corridor_elements[corridor.elements.as_usize_range()]
            .iter()
            .enumerate()
        {
            let (kind, ordinal) = match element {
                crate::lir::LirCorridorElement::RoadSection(ordinal) => {
                    (EntityKind::RoadSection, ordinal.raw())
                }
                crate::lir::LirCorridorElement::FacilityBand(ordinal) => {
                    (EntityKind::FacilityBand, ordinal.raw())
                }
            };
            push_relation_tuple(
                &mut relations,
                lir,
                EntityKind::RoadCorridor,
                corridor.ordinal.raw(),
                2,
                u32::try_from(local_index).expect("compile limits cap relation counts at u32"),
                kind,
                ordinal,
            );
        }
    }
    macro_rules! append_vector_relations {
        ($owners:expr, $range_field:ident, $values:expr, $owner_kind:expr, $role:expr, $subject_kind:expr) => {
            for owner in $owners {
                for (local_index, subject) in $values[owner.$range_field.as_usize_range()]
                    .iter()
                    .enumerate()
                {
                    push_relation_tuple(
                        &mut relations,
                        lir,
                        $owner_kind,
                        owner.ordinal.raw(),
                        $role,
                        u32::try_from(local_index)
                            .expect("compile limits cap relation counts at u32"),
                        $subject_kind,
                        subject.raw(),
                    );
                }
            }
        };
    }
    append_vector_relations!(
        &lir.road_sections,
        lanes,
        lir.road_section_lanes,
        EntityKind::RoadSection,
        3,
        EntityKind::AuthoringLane
    );
    append_vector_relations!(
        &lir.authoring_lanes,
        edge_chain,
        lir.authoring_lane_edges,
        EntityKind::AuthoringLane,
        4,
        EntityKind::LaneEdge
    );
    append_vector_relations!(
        &lir.lane_groups,
        members,
        lir.lane_group_members,
        EntityKind::LaneGroup,
        5,
        EntityKind::AuthoringLane
    );
    append_vector_relations!(
        &lir.junctions,
        movements,
        lir.junction_movements,
        EntityKind::Junction,
        6,
        EntityKind::Movement
    );
    append_vector_relations!(
        &lir.movements,
        maneuver_paths,
        lir.movement_maneuver_paths,
        EntityKind::Movement,
        7,
        EntityKind::ManeuverPath
    );
    append_vector_relations!(
        &lir.maneuver_paths,
        edges,
        lir.maneuver_path_edges,
        EntityKind::ManeuverPath,
        8,
        EntityKind::LaneEdge
    );
    let mut next_internal_index = vec![0_u32; lir.junctions.len()];
    for relation in &lir.junction_internal_edges {
        let local_index = next_internal_index[relation.junction.index()];
        next_internal_index[relation.junction.index()] += 1;
        push_relation_tuple(
            &mut relations,
            lir,
            EntityKind::Junction,
            relation.junction.raw(),
            9,
            local_index,
            EntityKind::LaneEdge,
            relation.edge.raw(),
        );
    }
    append_vector_relations!(
        &lir.maneuver_paths,
        maneuver_gates,
        lir.maneuver_path_gates,
        EntityKind::ManeuverPath,
        10,
        EntityKind::ManeuverGate
    );
    append_vector_relations!(
        &lir.maneuver_paths,
        waiting_zones,
        lir.maneuver_path_waiting_zones,
        EntityKind::ManeuverPath,
        11,
        EntityKind::WaitingZone
    );
    append_vector_relations!(
        &lir.stop_lines,
        maneuver_gates,
        lir.stop_line_maneuver_gates,
        EntityKind::StopLine,
        12,
        EntityKind::ManeuverGate
    );
    append_vector_relations!(
        &lir.static_routes,
        edges,
        lir.static_route_edges,
        EntityKind::StaticRoute,
        13,
        EntityKind::LaneEdge
    );
    for route in &lir.static_routes {
        for (index, occurrence) in lir.maneuver_occurrences
            [route.maneuver_occurrences.as_usize_range()]
        .iter()
        .enumerate()
        {
            push_relation_tuple(
                &mut relations,
                lir,
                EntityKind::StaticRoute,
                route.ordinal.raw(),
                14,
                u32::try_from(index).expect("compile limits cap occurrence counts at u32"),
                EntityKind::ManeuverPath,
                occurrence.maneuver_path.raw(),
            );
        }
        for (index, occurrence) in lir.gate_occurrences[route.gate_occurrences.as_usize_range()]
            .iter()
            .enumerate()
        {
            push_relation_tuple(
                &mut relations,
                lir,
                EntityKind::StaticRoute,
                route.ordinal.raw(),
                15,
                u32::try_from(index).expect("compile limits cap occurrence counts at u32"),
                EntityKind::ManeuverGate,
                occurrence.maneuver_gate.raw(),
            );
        }
        for (index, occurrence) in lir.waiting_zone_occurrences
            [route.waiting_zone_occurrences.as_usize_range()]
        .iter()
        .enumerate()
        {
            push_relation_tuple(
                &mut relations,
                lir,
                EntityKind::StaticRoute,
                route.ordinal.raw(),
                16,
                u32::try_from(index).expect("compile limits cap occurrence counts at u32"),
                EntityKind::WaitingZone,
                occurrence.waiting_zone.raw(),
            );
        }
    }
    append_vector_relations!(
        &lir.signal_controllers,
        signal_groups,
        lir.signal_controller_groups,
        EntityKind::SignalController,
        17,
        EntityKind::SignalGroup
    );
    append_vector_relations!(
        &lir.signal_controllers,
        phases,
        lir.signal_controller_phases,
        EntityKind::SignalController,
        18,
        EntityKind::SignalPhase
    );
    for gate in &lir.maneuver_gates {
        if let crate::lir::LirSignalControl::Group(group) = gate.signal_control {
            push_relation_tuple(
                &mut relations,
                lir,
                EntityKind::ManeuverGate,
                gate.ordinal.raw(),
                20,
                0,
                EntityKind::SignalGroup,
                group.raw(),
            );
        }
    }
    for space in &lir.parking_spaces {
        if let Some(area) = space.parking_area {
            push_relation_tuple(
                &mut relations,
                lir,
                EntityKind::ParkingSpace,
                space.ordinal.raw(),
                21,
                0,
                EntityKind::ParkingArea,
                area.raw(),
            );
        }
        for (role, edge) in [(22, space.entry.lane_edge), (23, space.exit.lane_edge)] {
            push_relation_tuple(
                &mut relations,
                lir,
                EntityKind::ParkingSpace,
                space.ordinal.raw(),
                role,
                0,
                EntityKind::LaneEdge,
                edge.raw(),
            );
        }
    }
    for class in &lir.participant_classes {
        if let Some(parent) = class.parent {
            push_relation_tuple(
                &mut relations,
                lir,
                EntityKind::ParticipantClass,
                class.ordinal.raw(),
                24,
                0,
                EntityKind::ParticipantClass,
                parent.raw(),
            );
        }
    }
    for rule in &lir.access_rules {
        let (target_kind, target_ordinal) = match rule.target {
            crate::lir::LirAccessTarget::LaneEdge(target) => (EntityKind::LaneEdge, target.raw()),
            crate::lir::LirAccessTarget::LaneGroup(target) => (EntityKind::LaneGroup, target.raw()),
            crate::lir::LirAccessTarget::RoadSection(target) => {
                (EntityKind::RoadSection, target.raw())
            }
            crate::lir::LirAccessTarget::ManeuverPath(target) => {
                (EntityKind::ManeuverPath, target.raw())
            }
        };
        push_relation_tuple(
            &mut relations,
            lir,
            EntityKind::AccessRule,
            rule.ordinal.raw(),
            25,
            0,
            target_kind,
            target_ordinal,
        );
        for (index, class) in lir.access_rule_participant_classes
            [rule.participant_classes.as_usize_range()]
        .iter()
        .enumerate()
        {
            push_relation_tuple(
                &mut relations,
                lir,
                EntityKind::AccessRule,
                rule.ordinal.raw(),
                26,
                u32::try_from(index).expect("compile limits cap relation counts at u32"),
                EntityKind::ParticipantClass,
                class.raw(),
            );
        }
    }
    for profile in &lir.vehicle_profiles {
        push_relation_tuple(
            &mut relations,
            lir,
            EntityKind::VehicleProfile,
            profile.ordinal.raw(),
            27,
            0,
            EntityKind::ParticipantClass,
            profile.participant_class.raw(),
        );
    }
    relations.sort_unstable();
    relations
}

fn genesis_relation_changes(lir: &crate::lir::LirUnit) -> Vec<OwnedRow> {
    canonical_relation_tuples(lir)
        .into_iter()
        .map(|relation| {
            row([
                field(1, OwnedValue::U8(0)),
                field(2, OwnedValue::U16(relation.owner_entity_kind.code())),
                field(3, OwnedValue::StableId128(relation.owner_stable_id)),
                field(4, OwnedValue::StableId128(relation.subject_stable_id)),
                field(5, OwnedValue::U8(relation.role)),
                field(8, OwnedValue::U32(relation.local_index)),
            ])
        })
        .collect()
}

fn canonical_geometry_value(
    row: RegistryCheckedRowView<'_>,
    frame_stable_id: [u8; 16],
    projected_tags: &[u16],
) -> Result<Box<[u8]>, PortableEmissionError> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&EntityKind::CanonicalFrame.code().to_le_bytes());
    bytes.extend_from_slice(&frame_stable_id);
    bytes.extend_from_slice(
        &u16::try_from(projected_tags.len())
            .map_err(|_| PortableEmissionError::ArithmeticOverflow)?
            .to_le_bytes(),
    );
    for tag in projected_tags {
        let value = row
            .field_by_tag(*tag)
            .ok_or(PortableEmissionError::InternalBindingMismatch)?
            .value_bytes();
        bytes.extend_from_slice(&tag.to_le_bytes());
        bytes.extend_from_slice(
            &u32::try_from(value.len())
                .map_err(|_| PortableEmissionError::ArithmeticOverflow)?
                .to_le_bytes(),
        );
        bytes.extend_from_slice(value);
    }
    Ok(bytes.into_boxed_slice())
}

fn genesis_geometry_changes(
    lir: &crate::lir::LirUnit,
    target: RegistryCheckedObjectView<'_>,
) -> Result<Vec<OwnedRow>, PortableEmissionError> {
    let section = target
        .section(4)
        .ok_or(PortableEmissionError::InternalBindingMismatch)?;
    let mut changes = Vec::new();
    for (table_index, (subject_kind, projected_tags)) in [
        (EntityKind::LaneEdge, &[3_u16, 4, 5, 6][..]),
        (EntityKind::FacilityBand, &[3_u16, 4][..]),
    ]
    .into_iter()
    .enumerate()
    {
        let table = section
            .table(
                u32::try_from(table_index + 1)
                    .expect("the canonical spatial registry contains only three tables"),
            )
            .ok_or(PortableEmissionError::InternalBindingMismatch)?;
        for row_index in 0..table.row_count() {
            let geometry = table
                .row(row_index)
                .ok_or(PortableEmissionError::InternalBindingMismatch)?;
            let subject_ordinal = checked_u32(geometry, 1)?;
            let frame_ordinal = checked_u32(geometry, 2)?;
            changes.push((
                subject_kind,
                entity_stable_id(lir, subject_kind, subject_ordinal),
                canonical_geometry_value(
                    geometry,
                    entity_stable_id(lir, EntityKind::CanonicalFrame, frame_ordinal),
                    projected_tags,
                )?,
            ));
        }
    }
    changes.sort_unstable_by_key(|(kind, stable_id, _)| (*kind, *stable_id));
    Ok(changes
        .into_iter()
        .map(|(kind, stable_id, bytes)| {
            row([
                field(1, OwnedValue::U8(0)),
                field(2, OwnedValue::U16(kind.code())),
                field(4, OwnedValue::StableId128(stable_id)),
                field(10, OwnedValue::Bytes(bytes)),
            ])
        })
        .collect())
}

fn build_lfca(
    output: &CompilationOutput,
    provenance: &PortableEmissionProvenanceV1,
    source_collection_digest: [u8; 32],
    declared_network_revision: [u8; 32],
) -> OwnedObject {
    let lir = output.lir().unit();
    let direction_profile = lir.geometry_profiles.map_or(0, |profiles| {
        geometry_direction_profile_code(profiles.direction)
    });
    let accuracy_profile = lir.geometry_profiles.map_or(0, |profiles| {
        geometry_accuracy_profile_code(profiles.accuracy)
    });
    let spatial_present = u8::from(
        lir.geometry_profiles.is_some()
            || !lir.canonical_frames.is_empty()
            || !lir.lane_edge_geometries.is_empty()
            || !lir.facility_band_geometries.is_empty(),
    );

    OwnedObject {
        kind: PortableObjectKind::CanonicalArtifact,
        sections: vec![
            section(
                1,
                [table(
                    1,
                    [row([
                        field(1, OwnedValue::U16(CANONICAL_ARTIFACT_FORMAT_VERSION)),
                        field(2, OwnedValue::U16(IDENTITY_ENCODING_VERSION)),
                        field(3, OwnedValue::U16(IDENTITY_REGISTRY_REVISION)),
                        field(4, OwnedValue::U16(NETWORK_REVISION_DERIVATION_VERSION)),
                        field(5, OwnedValue::U16(CONSTRAINT_CONTRACT_VERSION_V1)),
                        field(6, OwnedValue::U16(STATIC_EXECUTION_CONTRACT_VERSION_V1)),
                    ])],
                )],
            ),
            section(2, [table(1, canonical_identity_rows(lir))]),
            section(3, canonical_entity_tables(lir)),
            section(4, canonical_relation_tables(lir)),
            section(
                5,
                [
                    table(
                        1,
                        [row([
                            field(1, OwnedValue::U8(spatial_present)),
                            field(2, OwnedValue::U8(direction_profile)),
                        ])],
                    ),
                    table(2, lane_edge_geometry_rows(output, direction_profile)),
                    table(3, facility_band_geometry_rows(output, direction_profile)),
                ],
            ),
            section(
                6,
                [table(
                    1,
                    [row([
                        field(1, OwnedValue::U16(STATIC_EXECUTION_CONTRACT_VERSION_V1)),
                        field(2, OwnedValue::U16(CONSTRAINT_CONTRACT_VERSION_V1)),
                    ])],
                )],
            ),
            section(
                7,
                [table(
                    1,
                    [row([
                        field(1, OwnedValue::Utf8(provenance.compiler_build_id.clone())),
                        field(2, OwnedValue::U16(SOURCE_COLLECTION_DIGEST_VERSION_V1)),
                        field(3, OwnedValue::Sha256(source_collection_digest)),
                        field(4, OwnedValue::Sha256(PORTABLE_COMPILE_OPTIONS_DIGEST_V1)),
                        field(5, OwnedValue::U16(EMITTER_VERSION_V1)),
                        field(6, OwnedValue::U8(accuracy_profile)),
                    ])],
                )],
            ),
            section(
                8,
                [table(
                    1,
                    [row([field(
                        1,
                        OwnedValue::Sha256(declared_network_revision),
                    )])],
                )],
            ),
        ]
        .into_boxed_slice(),
    }
}

fn stable_id_bytes<K: EntityKindMarker>(id: StableId<K>) -> [u8; 16] {
    id.into_untyped().into_bytes()
}

fn ordinals<K: OrdinalKind + Copy>(values: &[Ordinal<K>]) -> Box<[u32]> {
    values.iter().map(|value| value.raw()).collect()
}

fn identity_fields(
    lir: &crate::lir::LirUnit,
    range: crate::arena::TableRange<crate::lir::LirIdentityField>,
) -> OwnedValue {
    let rows = lir.identity_fields[range.as_usize_range()]
        .iter()
        .map(|identity| {
            row([
                field(1, OwnedValue::U16(identity.tag.code())),
                field(
                    2,
                    OwnedValue::Bytes(
                        lir.identity_field_bytes[identity.value_bytes.as_usize_range()]
                            .to_vec()
                            .into_boxed_slice(),
                    ),
                ),
            ])
        })
        .collect();
    OwnedValue::RecordVector(rows)
}

fn canonical_identity_rows(lir: &crate::lir::LirUnit) -> Vec<OwnedRow> {
    let mut rows = Vec::new();
    macro_rules! append {
        ($kind:expr, $records:expr) => {
            rows.extend($records.iter().map(|record| {
                row([
                    field(1, OwnedValue::U16($kind.code())),
                    field(2, OwnedValue::U32(record.ordinal.raw())),
                    field(
                        3,
                        OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
                    ),
                    field(4, identity_fields(lir, record.identity_fields)),
                ])
            }));
        };
    }
    append!(EntityKind::RoadCorridor, lir.road_corridors);
    append!(EntityKind::RoadSection, lir.road_sections);
    append!(EntityKind::AuthoringLane, lir.authoring_lanes);
    append!(EntityKind::LaneEdge, lir.lane_edges);
    append!(EntityKind::Junction, lir.junctions);
    append!(EntityKind::Movement, lir.movements);
    append!(EntityKind::ManeuverPath, lir.maneuver_paths);
    append!(EntityKind::ManeuverGate, lir.maneuver_gates);
    append!(EntityKind::WaitingZone, lir.waiting_zones);
    append!(EntityKind::StopLine, lir.stop_lines);
    append!(EntityKind::SignalGroup, lir.signal_groups);
    append!(EntityKind::SignalController, lir.signal_controllers);
    append!(EntityKind::SignalPhase, lir.signal_phases);
    append!(EntityKind::ParkingArea, lir.parking_areas);
    append!(EntityKind::ParkingSpace, lir.parking_spaces);
    append!(EntityKind::LaneGroup, lir.lane_groups);
    append!(EntityKind::FacilityBand, lir.facility_bands);
    append!(EntityKind::ParticipantClass, lir.participant_classes);
    append!(EntityKind::AccessRule, lir.access_rules);
    append!(EntityKind::VehicleProfile, lir.vehicle_profiles);
    append!(EntityKind::StaticRoute, lir.static_routes);
    append!(EntityKind::CanonicalFrame, lir.canonical_frames);
    rows
}

fn canonical_entity_tables(lir: &crate::lir::LirUnit) -> Vec<OwnedTable> {
    let internal_edges: Vec<bool> = (0..lir.lane_edges.len())
        .map(|ordinal| {
            lir.junction_internal_edges
                .binary_search_by_key(
                    &u32::try_from(ordinal).expect("compile limits cap entity counts at u32"),
                    |entry| entry.edge.raw(),
                )
                .is_ok()
        })
        .collect();

    let road_corridors = lir.road_corridors.iter().map(|record| {
        let elements = lir.corridor_elements[record.elements.as_usize_range()]
            .iter()
            .map(|element| match element {
                crate::lir::LirCorridorElement::RoadSection(ordinal) => row([
                    field(1, OwnedValue::U8(0)),
                    field(2, OwnedValue::U32(ordinal.raw())),
                ]),
                crate::lir::LirCorridorElement::FacilityBand(ordinal) => row([
                    field(1, OwnedValue::U8(1)),
                    field(2, OwnedValue::U32(ordinal.raw())),
                ]),
            })
            .collect();
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::U32(record.reference_section.raw())),
            field(4, OwnedValue::RecordVector(elements)),
        ])
    });
    let road_sections = lir.road_sections.iter().map(|record| {
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::U32(record.road_corridor.raw())),
            field(4, OwnedValue::Utf8(record.kind_id.clone())),
            field(
                5,
                OwnedValue::OrdinalVectorU32(ordinals(
                    &lir.road_section_lanes[record.lanes.as_usize_range()],
                )),
            ),
        ])
    });
    let authoring_lanes = lir.authoring_lanes.iter().map(|record| {
        let mut fields = vec![
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::U32(record.road_section.raw())),
            field(
                4,
                OwnedValue::OrdinalVectorU32(ordinals(
                    &lir.authoring_lane_edges[record.edge_chain.as_usize_range()],
                )),
            ),
        ];
        if let Some(group) = record.lane_group {
            fields.push(field(5, OwnedValue::U32(group.raw())));
        }
        row(fields)
    });
    let lane_edges = lir.lane_edges.iter().map(|record| {
        let successors = lir.lane_edge_successors[record.successors.as_usize_range()]
            .iter()
            .copied()
            .filter(|successor| {
                !internal_edges[record.ordinal.index()] && !internal_edges[successor.index()]
            })
            .map(|ordinal| ordinal.raw())
            .collect();
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::F64(record.length_meters)),
            field(4, OwnedValue::F64(record.speed_limit_meters_per_second)),
            field(5, OwnedValue::OrdinalVectorU32(successors)),
        ])
    });
    let junctions = lir.junctions.iter().map(|record| {
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(
                3,
                OwnedValue::OrdinalVectorU32(ordinals(
                    &lir.junction_movements[record.movements.as_usize_range()],
                )),
            ),
        ])
    });
    let movements = lir.movements.iter().map(|record| {
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::U32(record.junction.raw())),
            field(
                4,
                OwnedValue::Utf8(record.directed_entry_approach_key.clone()),
            ),
            field(
                5,
                OwnedValue::Utf8(record.directed_exit_approach_key.clone()),
            ),
            field(
                6,
                OwnedValue::OrdinalVectorU32(ordinals(
                    &lir.movement_maneuver_paths[record.maneuver_paths.as_usize_range()],
                )),
            ),
        ])
    });
    let maneuver_paths = lir.maneuver_paths.iter().map(|record| {
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::U32(record.movement.raw())),
            field(
                4,
                OwnedValue::OrdinalVectorU32(ordinals(
                    &lir.maneuver_path_edges[record.edges.as_usize_range()],
                )),
            ),
            field(
                5,
                OwnedValue::OrdinalVectorU32(ordinals(
                    &lir.maneuver_path_gates[record.maneuver_gates.as_usize_range()],
                )),
            ),
            field(
                6,
                OwnedValue::OrdinalVectorU32(ordinals(
                    &lir.maneuver_path_waiting_zones[record.waiting_zones.as_usize_range()],
                )),
            ),
        ])
    });
    let maneuver_gates = lir.maneuver_gates.iter().map(|record| {
        let mut fields = vec![
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::U32(record.maneuver_path.raw())),
            field(4, OwnedValue::U32(record.transition_index)),
            field(5, OwnedValue::U32(record.stop_line.raw())),
        ];
        match record.signal_control {
            crate::lir::LirSignalControl::None => fields.push(field(6, OwnedValue::U8(0))),
            crate::lir::LirSignalControl::Group(group) => {
                fields.push(field(6, OwnedValue::U8(1)));
                fields.push(field(7, OwnedValue::U32(group.raw())));
            }
        }
        row(fields)
    });
    let waiting_zones = lir.waiting_zones.iter().map(|record| {
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::U32(record.maneuver_path.raw())),
            field(4, OwnedValue::U32(record.entry_gate.raw())),
            field(5, OwnedValue::U32(record.release_gate.raw())),
            field(6, OwnedValue::U32(record.max_occupancy)),
        ])
    });
    let stop_lines = lir.stop_lines.iter().map(|record| {
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::U32(record.lane_edge.raw())),
            field(
                4,
                OwnedValue::OrdinalVectorU32(ordinals(
                    &lir.stop_line_maneuver_gates[record.maneuver_gates.as_usize_range()],
                )),
            ),
        ])
    });
    let signal_groups = lir.signal_groups.iter().map(|record| {
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::U32(record.controller.raw())),
            field(
                4,
                OwnedValue::OrdinalVectorU32(ordinals(
                    &lir.signal_group_maneuver_gates[record.maneuver_gates.as_usize_range()],
                )),
            ),
        ])
    });
    let signal_controllers = lir.signal_controllers.iter().map(|record| {
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::U64(record.offset_ms)),
            field(4, OwnedValue::U64(record.cycle_duration_ms)),
            field(
                5,
                OwnedValue::OrdinalVectorU32(ordinals(
                    &lir.signal_controller_groups[record.signal_groups.as_usize_range()],
                )),
            ),
            field(
                6,
                OwnedValue::OrdinalVectorU32(ordinals(
                    &lir.signal_controller_phases[record.phases.as_usize_range()],
                )),
            ),
        ])
    });
    let signal_phases = lir.signal_phases.iter().map(|record| {
        let states = lir.signal_phase_states[record.states.as_usize_range()]
            .iter()
            .map(|state| {
                let aspect = match state.aspect {
                    laneflow_static_contract::SignalAspect::Red => 0,
                    laneflow_static_contract::SignalAspect::Yellow => 1,
                    laneflow_static_contract::SignalAspect::Green => 2,
                    _ => unreachable!("validated LIR only stores the closed v1 signal aspects"),
                };
                row([
                    field(1, OwnedValue::U32(state.signal_group.raw())),
                    field(2, OwnedValue::U8(aspect)),
                ])
            })
            .collect();
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::U32(record.controller.raw())),
            field(4, OwnedValue::U64(record.duration_ms)),
            field(5, OwnedValue::RecordVector(states)),
        ])
    });
    let parking_areas = lir.parking_areas.iter().map(|record| {
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(
                3,
                OwnedValue::OrdinalVectorU32(ordinals(
                    &lir.parking_area_spaces[record.parking_spaces.as_usize_range()],
                )),
            ),
        ])
    });
    let parking_spaces = lir.parking_spaces.iter().map(|record| {
        let mut fields = vec![
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
        ];
        if let Some(area) = record.parking_area {
            fields.push(field(3, OwnedValue::U32(area.raw())));
        }
        fields.extend([
            field(4, OwnedValue::U32(record.entry.lane_edge.raw())),
            field(5, OwnedValue::F64(record.entry.progress_meters)),
            field(6, OwnedValue::U32(record.exit.lane_edge.raw())),
            field(7, OwnedValue::F64(record.exit.progress_meters)),
            field(8, OwnedValue::F64(record.geometry.lateral_offset_meters)),
            field(9, OwnedValue::F64(record.geometry.heading_offset_radians)),
            field(10, OwnedValue::F64(record.geometry.length_meters)),
            field(11, OwnedValue::F64(record.geometry.width_meters)),
        ]);
        row(fields)
    });
    let lane_groups = lir.lane_groups.iter().map(|record| {
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::U32(record.road_section.raw())),
            field(
                4,
                OwnedValue::OrdinalVectorU32(ordinals(
                    &lir.lane_group_members[record.members.as_usize_range()],
                )),
            ),
        ])
    });
    let facility_bands = lir.facility_bands.iter().map(|record| {
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::U32(record.road_corridor.raw())),
            field(4, OwnedValue::Utf8(record.kind_id.clone())),
        ])
    });
    let participant_classes = lir.participant_classes.iter().map(|record| {
        let mut fields = vec![
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
        ];
        if let Some(parent) = record.parent {
            fields.push(field(3, OwnedValue::U32(parent.raw())));
        }
        fields.extend([
            field(4, OwnedValue::U32(record.depth)),
            field(5, OwnedValue::U32(record.subtree_enter)),
            field(6, OwnedValue::U32(record.subtree_exit)),
        ]);
        row(fields)
    });
    let access_rules = lir.access_rules.iter().map(|record| {
        let (target_kind, target_ordinal) = match record.target {
            crate::lir::LirAccessTarget::LaneEdge(ordinal) => (0, ordinal.raw()),
            crate::lir::LirAccessTarget::LaneGroup(ordinal) => (1, ordinal.raw()),
            crate::lir::LirAccessTarget::RoadSection(ordinal) => (2, ordinal.raw()),
            crate::lir::LirAccessTarget::ManeuverPath(ordinal) => (3, ordinal.raw()),
        };
        let effect = match record.effect {
            laneflow_static_contract::AccessEffect::Deny => 0,
            laneflow_static_contract::AccessEffect::Allow => 1,
            _ => unreachable!("validated LIR only stores the closed v1 access effects"),
        };
        let mut fields = vec![
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::U8(target_kind)),
            field(4, OwnedValue::U32(target_ordinal)),
            field(5, OwnedValue::U8(effect)),
            field(
                6,
                OwnedValue::OrdinalVectorU32(ordinals(
                    &lir.access_rule_participant_classes
                        [record.participant_classes.as_usize_range()],
                )),
            ),
        ];
        if let Some(regulation) = &record.regulation {
            let mut regulation_fields = vec![
                field(1, OwnedValue::Utf8(regulation.jurisdiction.clone())),
                field(2, OwnedValue::Utf8(regulation.version.clone())),
            ];
            if let Some(source) = &regulation.source {
                regulation_fields.push(field(3, OwnedValue::Utf8(source.clone())));
            }
            fields.push(field(
                7,
                OwnedValue::RecordVector(vec![row(regulation_fields)].into_boxed_slice()),
            ));
        }
        fields.push(field(8, OwnedValue::I32(record.priority)));
        row(fields)
    });
    let vehicle_profiles = lir.vehicle_profiles.iter().map(|record| {
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(3, OwnedValue::U32(record.participant_class.raw())),
            field(4, OwnedValue::F64(record.length_meters)),
            field(5, OwnedValue::F64(record.desired_speed_meters_per_second)),
            field(6, OwnedValue::F64(record.min_gap_meters)),
            field(7, OwnedValue::F64(record.time_headway_seconds)),
            field(
                8,
                OwnedValue::F64(record.max_acceleration_meters_per_second_squared),
            ),
            field(
                9,
                OwnedValue::F64(record.comfortable_deceleration_meters_per_second_squared),
            ),
            field(
                10,
                OwnedValue::F64(record.emergency_deceleration_meters_per_second_squared),
            ),
        ])
    });
    let static_routes = lir.static_routes.iter().map(|record| {
        let transition_gates = lir.static_route_transitions[record.transitions.as_usize_range()]
            .iter()
            .map(|transition| {
                let fields = transition
                    .maneuver_gate
                    .map(|gate| vec![field(1, OwnedValue::U32(gate.raw()))])
                    .unwrap_or_default();
                row(fields)
            })
            .collect();
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
            field(
                3,
                OwnedValue::OrdinalVectorU32(ordinals(
                    &lir.static_route_edges[record.edges.as_usize_range()],
                )),
            ),
            field(4, OwnedValue::RecordVector(transition_gates)),
        ])
    });
    let canonical_frames = lir.canonical_frames.iter().map(|record| {
        row([
            field(1, OwnedValue::U32(record.ordinal.raw())),
            field(
                2,
                OwnedValue::StableId128(stable_id_bytes(record.stable_id)),
            ),
        ])
    });

    vec![
        table(1, road_corridors),
        table(2, road_sections),
        table(3, authoring_lanes),
        table(4, lane_edges),
        table(5, junctions),
        table(6, movements),
        table(7, maneuver_paths),
        table(8, maneuver_gates),
        table(9, waiting_zones),
        table(10, stop_lines),
        table(11, signal_groups),
        table(12, signal_controllers),
        table(13, signal_phases),
        table(14, parking_areas),
        table(15, parking_spaces),
        table(16, lane_groups),
        table(17, facility_bands),
        table(18, participant_classes),
        table(19, access_rules),
        table(20, vehicle_profiles),
        table(21, static_routes),
        table(22, canonical_frames),
    ]
}

fn canonical_relation_tables(lir: &crate::lir::LirUnit) -> Vec<OwnedTable> {
    let junction_internal_edges = lir.junction_internal_edges.iter().map(|record| {
        row([
            field(1, OwnedValue::U32(record.edge.raw())),
            field(2, OwnedValue::U32(record.junction.raw())),
        ])
    });
    let mut maneuver_occurrences = Vec::new();
    let mut gate_occurrences = Vec::new();
    let mut waiting_occurrences = Vec::new();
    for route in &lir.static_routes {
        for (occurrence_index, occurrence) in lir.maneuver_occurrences
            [route.maneuver_occurrences.as_usize_range()]
        .iter()
        .enumerate()
        {
            maneuver_occurrences.push(row([
                field(1, OwnedValue::U32(route.ordinal.raw())),
                field(
                    2,
                    OwnedValue::U32(
                        u32::try_from(occurrence_index)
                            .expect("compile limits cap occurrence counts at u32"),
                    ),
                ),
                field(3, OwnedValue::U32(occurrence.maneuver_path.raw())),
                field(4, OwnedValue::U32(occurrence.entry_route_edge_index)),
                field(5, OwnedValue::U32(occurrence.exit_route_edge_index)),
                field(
                    6,
                    OwnedValue::U32(
                        occurrence
                            .gate_occurrences
                            .start()
                            .saturating_sub(route.gate_occurrences.start()),
                    ),
                ),
                field(7, OwnedValue::U32(occurrence.gate_occurrences.len())),
                field(
                    8,
                    OwnedValue::U32(
                        occurrence
                            .waiting_zone_occurrences
                            .start()
                            .saturating_sub(route.waiting_zone_occurrences.start()),
                    ),
                ),
                field(
                    9,
                    OwnedValue::U32(occurrence.waiting_zone_occurrences.len()),
                ),
            ]));
        }
        for (occurrence_index, occurrence) in lir.gate_occurrences
            [route.gate_occurrences.as_usize_range()]
        .iter()
        .enumerate()
        {
            let mut fields = vec![
                field(1, OwnedValue::U32(route.ordinal.raw())),
                field(
                    2,
                    OwnedValue::U32(
                        u32::try_from(occurrence_index)
                            .expect("compile limits cap occurrence counts at u32"),
                    ),
                ),
                field(3, OwnedValue::U32(occurrence.maneuver_gate.raw())),
                field(4, OwnedValue::U32(occurrence.maneuver_occurrence_index)),
                field(5, OwnedValue::U32(occurrence.from_route_edge_index)),
            ];
            if let Some(next) = occurrence.next_gate_occurrence_index {
                fields.push(field(6, OwnedValue::U32(next)));
            }
            fields.push(field(
                7,
                OwnedValue::U32(occurrence.next_boundary_route_edge_index),
            ));
            if let Some(waiting) = occurrence.waiting_zone_occurrence_index {
                fields.push(field(8, OwnedValue::U32(waiting)));
            }
            gate_occurrences.push(row(fields));
        }
        for (occurrence_index, occurrence) in lir.waiting_zone_occurrences
            [route.waiting_zone_occurrences.as_usize_range()]
        .iter()
        .enumerate()
        {
            waiting_occurrences.push(row([
                field(1, OwnedValue::U32(route.ordinal.raw())),
                field(
                    2,
                    OwnedValue::U32(
                        u32::try_from(occurrence_index)
                            .expect("compile limits cap occurrence counts at u32"),
                    ),
                ),
                field(3, OwnedValue::U32(occurrence.waiting_zone.raw())),
                field(4, OwnedValue::U32(occurrence.maneuver_occurrence_index)),
                field(5, OwnedValue::U32(occurrence.entry_gate_occurrence_index)),
                field(6, OwnedValue::U32(occurrence.release_gate_occurrence_index)),
                field(7, OwnedValue::U32(occurrence.entry_route_edge_index)),
                field(8, OwnedValue::U32(occurrence.release_route_edge_index)),
            ]));
        }
    }

    let mut reverse_index = Vec::new();
    macro_rules! append_reverse {
        ($kind:expr, $records:expr, $occurrences:expr) => {
            for record in $records {
                for occurrence in &$occurrences[record.static_route_occurrences.as_usize_range()] {
                    reverse_index.push(row([
                        field(1, OwnedValue::U16($kind.code())),
                        field(2, OwnedValue::U32(record.ordinal.raw())),
                        field(3, OwnedValue::U32(occurrence.static_route.raw())),
                        field(4, OwnedValue::U32(occurrence.occurrence_index)),
                    ]));
                }
            }
        };
    }
    append_reverse!(
        EntityKind::LaneEdge,
        &lir.lane_edges,
        lir.lane_edge_route_occurrences
    );
    append_reverse!(
        EntityKind::ManeuverPath,
        &lir.maneuver_paths,
        lir.maneuver_path_route_occurrences
    );
    append_reverse!(
        EntityKind::ManeuverGate,
        &lir.maneuver_gates,
        lir.maneuver_gate_route_occurrences
    );
    append_reverse!(
        EntityKind::WaitingZone,
        &lir.waiting_zones,
        lir.waiting_zone_route_occurrences
    );

    vec![
        table(1, junction_internal_edges),
        table(2, maneuver_occurrences),
        table(3, gate_occurrences),
        table(4, waiting_occurrences),
        table(5, reverse_index),
    ]
}

fn geometry_relation_flags(output: &CompilationOutput) -> BTreeMap<(u32, u8, u32), bool> {
    output
        .source_map_input()
        .spatial_relation_sources()
        .map(|source| {
            (
                (
                    source.owner_ordinal().raw(),
                    source_relation_role_code(source.role()),
                    source.local_index(),
                ),
                source.geometry_source_ranges().len() != 0,
            )
        })
        .collect()
}

fn point_rows(points: &[crate::lir::LirCanonicalPoint3F32]) -> Box<[OwnedRow]> {
    points
        .iter()
        .map(|point| {
            row([
                field(1, OwnedValue::F32(point.x)),
                field(2, OwnedValue::F32(point.y)),
                field(3, OwnedValue::F32(point.z)),
            ])
        })
        .collect()
}

fn lane_edge_geometry_rows(output: &CompilationOutput, direction_profile: u8) -> Vec<OwnedRow> {
    let lir = output.lir().unit();
    let flags = geometry_relation_flags(output);
    let mut next_local_index_by_frame = vec![0_u32; lir.canonical_frames.len()];
    lir.lane_edge_geometries
        .iter()
        .enumerate()
        .map(|(lane_edge, geometry)| {
            let frame = geometry.canonical_frame.raw();
            let local_index = next_local_index_by_frame[geometry.canonical_frame.index()];
            next_local_index_by_frame[geometry.canonical_frame.index()] += 1;
            let applies = flags
                .get(&(
                    frame,
                    source_relation_role_code(
                        crate::source_map::SourceRelationRole::CanonicalFrameLaneEdgeGeometry,
                    ),
                    local_index,
                ))
                .copied()
                .unwrap_or(false);
            debug_assert!(direction_profile != 0 || !applies);
            let segments = lir.spatial_segments[geometry.segments.as_usize_range()]
                .iter()
                .map(|segment| {
                    row([
                        field(1, OwnedValue::F32(segment.length_meters)),
                        field(2, OwnedValue::F32(segment.cumulative_end_meters)),
                        field(3, OwnedValue::F32(segment.tangent[0])),
                        field(4, OwnedValue::F32(segment.tangent[1])),
                        field(5, OwnedValue::F32(segment.tangent[2])),
                        field(6, OwnedValue::F32(segment.up[0])),
                        field(7, OwnedValue::F32(segment.up[1])),
                        field(8, OwnedValue::F32(segment.up[2])),
                    ])
                })
                .collect();
            row([
                field(
                    1,
                    OwnedValue::U32(
                        u32::try_from(lane_edge)
                            .expect("compile limits cap geometry counts at u32"),
                    ),
                ),
                field(2, OwnedValue::U32(frame)),
                field(3, OwnedValue::F32(geometry.arc_length_meters)),
                field(
                    4,
                    OwnedValue::RecordVector(point_rows(
                        &lir.canonical_points[geometry.points.as_usize_range()],
                    )),
                ),
                field(5, OwnedValue::RecordVector(segments)),
                field(6, OwnedValue::U8(u8::from(applies))),
            ])
        })
        .collect()
}

fn facility_band_geometry_rows(output: &CompilationOutput, direction_profile: u8) -> Vec<OwnedRow> {
    let lir = output.lir().unit();
    let flags = geometry_relation_flags(output);
    let mut next_local_index_by_frame = vec![0_u32; lir.canonical_frames.len()];
    lir.facility_band_geometries
        .iter()
        .map(|geometry| {
            let frame = geometry.canonical_frame.raw();
            let local_index = next_local_index_by_frame[geometry.canonical_frame.index()];
            next_local_index_by_frame[geometry.canonical_frame.index()] += 1;
            let applies = flags
                .get(&(
                    frame,
                    source_relation_role_code(
                        crate::source_map::SourceRelationRole::CanonicalFrameFacilityBandGeometry,
                    ),
                    local_index,
                ))
                .copied()
                .unwrap_or(false);
            debug_assert!(direction_profile != 0 || !applies);
            row([
                field(1, OwnedValue::U32(geometry.facility_band.raw())),
                field(2, OwnedValue::U32(frame)),
                field(
                    3,
                    OwnedValue::RecordVector(point_rows(
                        &lir.canonical_points[geometry.points.as_usize_range()],
                    )),
                ),
                field(4, OwnedValue::U8(u8::from(applies))),
            ])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relation(role: u8, local_index: u32, subject: u8) -> RelationTuple {
        let mut owner_stable_id = [0_u8; 16];
        owner_stable_id[15] = 1;
        let mut subject_stable_id = [0_u8; 16];
        subject_stable_id[15] = subject;
        RelationTuple {
            owner_entity_kind: EntityKind::StaticRoute,
            owner_stable_id,
            role,
            local_index,
            subject_entity_kind: EntityKind::LaneEdge,
            subject_stable_id,
        }
    }

    #[test]
    fn set_pairing_ignores_canonical_position_only_changes() {
        let mut changes = Vec::new();
        pair_set_relations(&[relation(1, 1, 7)], &[relation(1, 0, 7)], &mut changes).unwrap();
        assert!(changes.is_empty());
    }

    #[test]
    fn scalar_pairing_emits_one_reconnect() {
        let mut changes = Vec::new();
        pair_scalar_relations(&[relation(20, 0, 7)], &[relation(20, 0, 8)], &mut changes).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].change_kind, 3);
        assert_eq!(
            changes[0].before_target,
            Some(relation(20, 0, 7).subject_stable_id)
        );
        assert_eq!(
            changes[0].after_target,
            Some(relation(20, 0, 8).subject_stable_id)
        );
    }

    #[test]
    fn occurrence_pairing_uses_same_subject_rank() {
        let mut changes = Vec::new();
        pair_domain_relations(
            &[relation(13, 0, 7), relation(13, 2, 7)],
            &[relation(13, 1, 7), relation(13, 2, 7)],
            &mut changes,
        );
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].change_kind, 2);
        assert_eq!(changes[0].before_local_index, Some(0));
        assert_eq!(changes[0].after_local_index, Some(1));
    }
}
