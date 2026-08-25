//! 附录 A 的字段值域与对象内直接绑定预检。
//!
//! 本层只消费已经完成 registry 结构预检的对象借用。它检查不需要外部对象或全局语义重算
//! 即可判定的闭合值域与同对象直接绑定；跨表排序/引用、Identity/NetworkRevision 重算、
//! diff 完备性和来源真实性不属于本层。

use core::str;

#[cfg(test)]
use laneflow_static_contract::FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES;
use laneflow_static_contract::{
    EntityKind, FORMAT_HARD_MAX_FIELDS_PER_ROW, FieldEncoding, FieldTag, HEADING_MINUS_PI_F32_BITS,
    HEADING_PLUS_PI_F32_BITS, MAX_ACCEL_METERS_PER_SECOND_SQUARED, MAX_LANE_EDGE_LENGTH_MM,
    MAX_MIN_GAP_MM, MAX_PARKING_LATERAL_OFFSET_ABS_MM, MAX_SPEED_MM_S, MAX_TIME_HEADWAY_SECONDS,
    MAX_VEHICLE_LENGTH_MM, MIN_ACCEL_METERS_PER_SECOND_SQUARED, MIN_LANE_EDGE_LENGTH_MM,
    MIN_PARKING_LATERAL_OFFSET_ABS_MM, MIN_SPEED_MM_S, MIN_VEHICLE_LENGTH_MM,
    PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM, PortableObjectKind, PortableTableSchema,
};

use crate::{
    FormatError, FormatLimits, FormatStructure, LimitDimension, RegistryCheckedObjectView,
    wire::{checked_slice, read_u8, read_u16, read_u32, read_u64},
};

const SECTION_HEADER_BYTES: u64 = 4;
const TABLE_HEADER_BYTES: u64 = 16;
const ROW_HEADER_BYTES: u64 = 16;
const FIELD_HEADER_BYTES: u64 = 12;
const MAX_FIELDS_PER_ROW: usize = FORMAT_HARD_MAX_FIELDS_PER_ROW as usize;
const MAX_PROPERTY_STEPS: usize = 4;
const MAX_SAFE_INTEGER_U64: u64 = 9_007_199_254_740_991;
const MAX_COMPILER_BUILD_ID_BYTES: usize = 128;
const PORTABLE_COMPILE_OPTIONS_DIGEST_V1: [u8; 32] = [
    0x32, 0x26, 0x82, 0xf4, 0x55, 0xd0, 0x6b, 0x36, 0xe9, 0xe3, 0x71, 0x9f, 0x34, 0x1d, 0xb3, 0x8f,
    0x3e, 0xcd, 0xa6, 0x1d, 0x52, 0xc5, 0x3d, 0x9d, 0x6f, 0xe3, 0xdc, 0xa5, 0x40, 0xee, 0xf4, 0x45,
];

/// 已完成 registry 结构预检和附录 A 直接值域检查的对象借用。
///
/// 该能力值证明字段已按登记类型解码，并且对象种类专用的封闭枚举、版本、token、
/// 同行存在性矩阵、局部向量基数、局部数值关系和 LFCP v2 对象键直接绑定有效。它不证明跨行/跨表引用、
/// 行排序键、StableId/NetworkRevision 重算、LFSD 完备性、跨对象摘要绑定或真实性，因而
/// 不是 `validated` 或 `trusted` view。
#[derive(Clone, Copy, Debug)]
pub struct ValueCheckedObjectView<'a> {
    registry: RegistryCheckedObjectView<'a>,
}

impl<'a> ValueCheckedObjectView<'a> {
    /// 已与 magic、registry 和字段值域一致的对象种类。
    #[must_use]
    pub const fn kind(self) -> PortableObjectKind {
        self.registry.kind()
    }

    /// 完成结构和值域预检的 exact bytes。
    #[must_use]
    pub const fn bytes(self) -> &'a [u8] {
        self.registry.bytes()
    }

    /// 保留已证明的 registry 能力，供只需要结构信息的调用方使用。
    #[must_use]
    pub const fn registry_view(self) -> RegistryCheckedObjectView<'a> {
        self.registry
    }
}

impl<'a> RegistryCheckedObjectView<'a> {
    /// 在既有完整 registry 结构证明之上检查对象种类专用的直接值域。
    pub fn check_value_domains(self) -> Result<ValueCheckedObjectView<'a>, FormatError> {
        validate_object_values(self, self.limits())?;
        Ok(ValueCheckedObjectView { registry: self })
    }
}

/// 对完整对象执行 registry 结构预检与直接值域预检。
pub fn preflight_object_values(
    bytes: &[u8],
    expected_kind: PortableObjectKind,
    limits: FormatLimits,
) -> Result<ValueCheckedObjectView<'_>, FormatError> {
    crate::object::preflight_object_registry(bytes, expected_kind, limits)?.check_value_domains()
}

#[derive(Clone, Copy)]
struct FieldRef<'a> {
    tag: u16,
    value_offset: u64,
    value: &'a [u8],
}

impl FieldRef<'_> {
    fn u8(self) -> Result<u8, FormatError> {
        read_u8(self.value, 0, FormatStructure::FieldValue)
    }

    fn u16(self) -> Result<u16, FormatError> {
        read_u16(self.value, 0, FormatStructure::FieldValue)
    }

    fn u32(self) -> Result<u32, FormatError> {
        read_u32(self.value, 0, FormatStructure::FieldValue)
    }

    fn u64(self) -> Result<u64, FormatError> {
        read_u64(self.value, 0, FormatStructure::FieldValue)
    }

    fn f32(self) -> Result<f32, FormatError> {
        Ok(f32::from_bits(self.u32()?))
    }

    fn i32(self) -> Result<i32, FormatError> {
        Ok(self.u32()? as i32)
    }
}

#[derive(Clone, Copy)]
struct RowRef<'a> {
    fields: [Option<FieldRef<'a>>; MAX_FIELDS_PER_ROW],
    field_count: usize,
}

impl<'a> RowRef<'a> {
    fn field(self, tag: u16) -> Option<FieldRef<'a>> {
        self.fields[..self.field_count]
            .iter()
            .flatten()
            .copied()
            .find(|field| field.tag == tag)
    }

    fn required(self, tag: u16) -> Result<FieldRef<'a>, FormatError> {
        self.field(tag).ok_or(FormatError::BindingMismatch {
            structure: FormatStructure::RowFields,
        })
    }

    fn has(self, tag: u16) -> bool {
        self.field(tag).is_some()
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct PropertyStep {
    kind: u8,
    container: u16,
    member: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SemanticDiffBaseKind {
    Genesis,
    Artifact,
}

struct DirectBindings {
    lfca_spatial_present: Option<u8>,
    lfca_direction_profile: Option<u8>,
    lfca_has_canonical_frame: bool,
    lfca_has_lane_edge_geometry: bool,
    lfca_has_facility_band_geometry: bool,
    lfsd_base_kind: Option<SemanticDiffBaseKind>,
    lfcp_artifact_digest: Option<[u8; 32]>,
    lfcp_source_map_digest: Option<[u8; 32]>,
    contract_format: u16,
}

impl Default for DirectBindings {
    fn default() -> Self {
        Self {
            lfca_spatial_present: None,
            lfca_direction_profile: None,
            lfca_has_canonical_frame: false,
            lfca_has_lane_edge_geometry: false,
            lfca_has_facility_band_geometry: false,
            lfsd_base_kind: None,
            lfcp_artifact_digest: None,
            lfcp_source_map_digest: None,
            contract_format: 1,
        }
    }
}

fn validate_object_values(
    view: RegistryCheckedObjectView<'_>,
    limits: FormatLimits,
) -> Result<(), FormatError> {
    let object_schema = view.schema();
    let mut bindings = DirectBindings {
        contract_format: view.contract_format(),
        ..DirectBindings::default()
    };
    for (section_index, section_schema) in object_schema.sections.iter().enumerate() {
        let ordinal =
            u32::try_from(section_index).map_err(|_| FormatError::ArithmeticOverflow {
                structure: FormatStructure::SectionDirectory,
            })?;
        let section = view.section(ordinal).ok_or(FormatError::BindingMismatch {
            structure: FormatStructure::Section,
        })?;
        let bytes = section.bytes();
        let mut cursor = SECTION_HEADER_BYTES;
        for table_schema in section_schema.tables {
            let rows_byte_length = read_u64(bytes, cursor + 8, FormatStructure::Table)?;
            let table_byte_length = TABLE_HEADER_BYTES.checked_add(rows_byte_length).ok_or(
                FormatError::ArithmeticOverflow {
                    structure: FormatStructure::Table,
                },
            )?;
            let table_bytes =
                checked_slice(bytes, cursor, table_byte_length, FormatStructure::Table)?;
            validate_table_values(
                view.kind(),
                section_schema.kind,
                table_schema,
                table_bytes,
                limits,
                &mut bindings,
            )?;
            cursor =
                cursor
                    .checked_add(table_byte_length)
                    .ok_or(FormatError::ArithmeticOverflow {
                        structure: FormatStructure::Section,
                    })?;
        }
    }
    validate_object_bindings(view.kind(), &bindings)
}

fn validate_table_values(
    object_kind: PortableObjectKind,
    section_kind: u16,
    table_schema: &PortableTableSchema,
    bytes: &[u8],
    limits: FormatLimits,
    bindings: &mut DirectBindings,
) -> Result<(), FormatError> {
    let row_count = read_u32(bytes, 4, FormatStructure::Table)?;
    record_table_bindings(
        object_kind,
        section_kind,
        table_schema.kind,
        row_count,
        bindings,
    )?;
    let mut cursor = TABLE_HEADER_BYTES;
    for _ in 0..row_count {
        let (row, end) = parse_row(bytes, cursor)?;
        match object_kind {
            PortableObjectKind::CanonicalArtifact => {
                validate_lfca_row(
                    section_kind,
                    table_schema.kind,
                    row,
                    limits.max_identity_ascii_bytes(),
                    bindings,
                )?;
            }
            PortableObjectKind::SourceMap => {
                validate_lfsm_row(
                    section_kind,
                    table_schema.kind,
                    row,
                    limits.max_identity_ascii_bytes(),
                    bindings.contract_format,
                )?;
            }
            PortableObjectKind::SemanticDiff => {
                validate_lfsd_row(section_kind, table_schema.kind, row, bindings)?;
            }
            PortableObjectKind::CanonicalPublicationDescriptor => {
                validate_lfcp_row(section_kind, row, bindings)?;
            }
        }
        cursor = end;
    }
    Ok(())
}

fn record_table_bindings(
    object_kind: PortableObjectKind,
    section_kind: u16,
    table_kind: u16,
    row_count: u32,
    bindings: &mut DirectBindings,
) -> Result<(), FormatError> {
    match (object_kind, section_kind, table_kind) {
        (PortableObjectKind::CanonicalArtifact, 3, 22) => {
            bindings.lfca_has_canonical_frame = row_count != 0;
        }
        (PortableObjectKind::CanonicalArtifact, 5, 2) => {
            bindings.lfca_has_lane_edge_geometry = row_count != 0;
        }
        (PortableObjectKind::CanonicalArtifact, 5, 3) => {
            bindings.lfca_has_facility_band_geometry = row_count != 0;
        }
        (PortableObjectKind::SemanticDiff, 5, 1)
            if bindings.lfsd_base_kind == Some(SemanticDiffBaseKind::Genesis) && row_count != 0 =>
        {
            return Err(table_binding_mismatch());
        }
        (PortableObjectKind::SemanticDiff, 6, 1) => {
            let base_kind = bindings.lfsd_base_kind.ok_or_else(table_binding_mismatch)?;
            if base_kind == SemanticDiffBaseKind::Genesis && row_count != 1 {
                return Err(table_binding_mismatch());
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_object_bindings(
    object_kind: PortableObjectKind,
    bindings: &DirectBindings,
) -> Result<(), FormatError> {
    if object_kind != PortableObjectKind::CanonicalArtifact {
        return Ok(());
    }
    let spatial_present = bindings
        .lfca_spatial_present
        .ok_or_else(table_binding_mismatch)?;
    let direction_profile = bindings
        .lfca_direction_profile
        .ok_or_else(table_binding_mismatch)?;
    let derived_spatial_present = direction_profile != 0
        || bindings.lfca_has_canonical_frame
        || bindings.lfca_has_lane_edge_geometry
        || bindings.lfca_has_facility_band_geometry;
    if (spatial_present != 0) != derived_spatial_present {
        return Err(table_binding_mismatch());
    }
    Ok(())
}

fn parse_row(bytes: &[u8], row_offset: u64) -> Result<(RowRef<'_>, u64), FormatError> {
    let row_byte_length = read_u64(bytes, row_offset, FormatStructure::Row)?;
    let row_end =
        row_offset
            .checked_add(row_byte_length)
            .ok_or(FormatError::ArithmeticOverflow {
                structure: FormatStructure::Row,
            })?;
    let field_count_u32 = read_u32(bytes, row_offset + 8, FormatStructure::Row)?;
    let field_count =
        usize::try_from(field_count_u32).map_err(|_| FormatError::ArithmeticOverflow {
            structure: FormatStructure::RowFields,
        })?;
    if field_count > MAX_FIELDS_PER_ROW {
        return Err(FormatError::LengthMismatch {
            structure: FormatStructure::RowFields,
            declared: u64::from(field_count_u32),
            actual: MAX_FIELDS_PER_ROW as u64,
        });
    }
    let mut fields = [None; MAX_FIELDS_PER_ROW];
    let mut cursor = row_offset + ROW_HEADER_BYTES;
    for slot in &mut fields[..field_count] {
        let tag = read_u16(bytes, cursor, FormatStructure::Field)?;
        let value_byte_length = read_u64(bytes, cursor + 4, FormatStructure::Field)?;
        let value_offset =
            cursor
                .checked_add(FIELD_HEADER_BYTES)
                .ok_or(FormatError::ArithmeticOverflow {
                    structure: FormatStructure::Field,
                })?;
        let value = checked_slice(
            bytes,
            value_offset,
            value_byte_length,
            FormatStructure::FieldValue,
        )?;
        *slot = Some(FieldRef {
            tag,
            value_offset,
            value,
        });
        cursor =
            value_offset
                .checked_add(value_byte_length)
                .ok_or(FormatError::ArithmeticOverflow {
                    structure: FormatStructure::Field,
                })?;
    }
    if cursor != row_end {
        return Err(FormatError::LengthMismatch {
            structure: FormatStructure::RowFields,
            declared: row_byte_length.saturating_sub(ROW_HEADER_BYTES),
            actual: cursor.saturating_sub(row_offset + ROW_HEADER_BYTES),
        });
    }
    Ok((
        RowRef {
            fields,
            field_count,
        },
        row_end,
    ))
}

fn visit_record_rows(
    field: FieldRef<'_>,
    mut visitor: impl FnMut(RowRef<'_>) -> Result<(), FormatError>,
) -> Result<u32, FormatError> {
    let count = read_u32(field.value, 0, FormatStructure::RecordVector)?;
    let mut cursor = 4;
    for _ in 0..count {
        let (row, end) = parse_row(field.value, cursor)?;
        visitor(row)?;
        cursor = end;
    }
    if cursor != field.value.len() as u64 {
        return Err(FormatError::LengthMismatch {
            structure: FormatStructure::RecordVector,
            declared: field.value.len() as u64,
            actual: cursor,
        });
    }
    Ok(count)
}

fn validate_lfca_row(
    section: u16,
    table: u16,
    row: RowRef<'_>,
    max_identity_ascii_bytes: u64,
    bindings: &mut DirectBindings,
) -> Result<(), FormatError> {
    if section == 3 {
        validate_lfca_entity_vector_cardinalities(table, row)?;
    }
    match (section, table) {
        (1, 1) => {
            require_exact_u16(row.required(1)?, bindings.contract_format)?;
            require_exact_u16(row.required(2)?, 1)?;
            require_exact_u16(row.required(3)?, 1)?;
            require_exact_u16(row.required(4)?, 1)?;
            require_exact_u16(row.required(5)?, bindings.contract_format)?;
            require_exact_u16(row.required(6)?, bindings.contract_format)?;
        }
        (2, 1) => {
            let entity = require_entity_kind(row.required(1)?)?;
            validate_identity_fields(entity, row.required(4)?, max_identity_ascii_bytes)?;
        }
        (3, 1) => {
            visit_record_rows(row.required(4)?, |nested| {
                require_u8_range(nested.required(1)?, 0, 1).map(|_| ())
            })?;
        }
        (3, 2) => validate_kind_id(row.required(4)?, true)?,
        (3, 4) => {
            require_u32_inclusive(
                row.required(3)?,
                MIN_LANE_EDGE_LENGTH_MM,
                MAX_LANE_EDGE_LENGTH_MM,
            )?;
            require_u32_inclusive(row.required(4)?, MIN_SPEED_MM_S, MAX_SPEED_MM_S)?;
        }
        (3, 8) => {
            let control = require_u8_range(row.required(6)?, 0, 1)?;
            if row.has(7) != (control == 1) {
                return Err(row_binding_mismatch());
            }
        }
        (3, 9) => require_u32_greater(row.required(6)?, 0)?,
        (3, 12) => {
            let offset = row.required(3)?.u64()?;
            let cycle = row.required(4)?.u64()?;
            if offset > MAX_SAFE_INTEGER_U64
                || cycle == 0
                || cycle > MAX_SAFE_INTEGER_U64
                || offset >= cycle
            {
                return Err(noncanonical(row.required(3)?));
            }
        }
        (3, 13) => {
            let duration = row.required(4)?;
            if duration.u64()? == 0 || duration.u64()? > MAX_SAFE_INTEGER_U64 {
                return Err(noncanonical(duration));
            }
            visit_record_rows(row.required(5)?, |nested| {
                require_u8_range(nested.required(2)?, 0, 2).map(|_| ())
            })?;
        }
        (3, 15) => validate_parking_space(row)?,
        (3, 17) => validate_kind_id(row.required(4)?, false)?,
        (3, 19) => {
            require_u8_range(row.required(3)?, 0, 3)?;
            require_u8_range(row.required(5)?, 0, 1)?;
            if let Some(regulation) = row.field(7) {
                let count = visit_record_rows(regulation, |nested| {
                    require_scalar_count(nested.required(1)?, 1, 128)?;
                    require_scalar_count(nested.required(2)?, 1, 128)?;
                    if let Some(source) = nested.field(3) {
                        require_scalar_count(source, 1, 128)?;
                    }
                    Ok(())
                })?;
                if count != 1 {
                    return Err(row_binding_mismatch());
                }
            }
        }
        (3, 20) => validate_vehicle_profile(row)?,
        (4, 5) => {
            let kind = row.required(1)?;
            if !matches!(kind.u16()?, 4 | 7 | 8 | 9) {
                return Err(unknown(kind, u64::from(kind.u16()?)));
            }
        }
        (5, 1) => {
            let present = require_u8_range(row.required(1)?, 0, 1)?;
            let profile = require_u8_range(row.required(2)?, 0, 3)?;
            if present == 0 && profile != 0 {
                return Err(row_binding_mismatch());
            }
            bindings.lfca_spatial_present = Some(present);
            bindings.lfca_direction_profile = Some(profile);
        }
        (5, 2) => {
            require_f32_greater(row.required(3)?, 0.1)?;
            validate_geometry_rows(row.required(4)?, Some(row.required(5)?))?;
            validate_direction_profile_applies(row.required(6)?, bindings)?;
        }
        (5, 3) => {
            validate_geometry_rows(row.required(3)?, None)?;
            validate_direction_profile_applies(row.required(4)?, bindings)?;
        }
        (6, 1) => {
            require_exact_u16(row.required(1)?, bindings.contract_format)?;
            require_exact_u16(row.required(2)?, bindings.contract_format)?;
        }
        (7, 1) => {
            validate_compiler_build_id(row.required(1)?)?;
            require_exact_u16(row.required(2)?, 1)?;
            require_exact_u16(row.required(5)?, 1)?;
            let accuracy_profile = require_u8_range(row.required(6)?, 0, 3)?;
            let direction_profile = bindings
                .lfca_direction_profile
                .ok_or_else(row_binding_mismatch)?;
            if (accuracy_profile == 0) != (direction_profile == 0) {
                return Err(row_binding_mismatch());
            }
            if row.required(4)?.value != PORTABLE_COMPILE_OPTIONS_DIGEST_V1 {
                return Err(row_binding_mismatch());
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_lfca_entity_vector_cardinalities(
    table: u16,
    row: RowRef<'_>,
) -> Result<(), FormatError> {
    match table {
        1 => {
            require_vector_count(row.required(4)?, FormatStructure::RecordVector, 1)?;
        }
        2 => {
            require_vector_count(row.required(5)?, FormatStructure::OrdinalVector, 1)?;
        }
        3 => {
            require_vector_count(row.required(4)?, FormatStructure::OrdinalVector, 1)?;
        }
        5 => {
            require_vector_count(row.required(3)?, FormatStructure::OrdinalVector, 1)?;
        }
        6 => {
            require_vector_count(row.required(6)?, FormatStructure::OrdinalVector, 1)?;
        }
        7 => {
            require_vector_count(row.required(4)?, FormatStructure::OrdinalVector, 2)?;
        }
        10 | 11 | 16 => {
            require_vector_count(row.required(4)?, FormatStructure::OrdinalVector, 1)?;
        }
        12 => {
            require_vector_count(row.required(5)?, FormatStructure::OrdinalVector, 1)?;
            require_vector_count(row.required(6)?, FormatStructure::OrdinalVector, 1)?;
        }
        13 => {
            require_vector_count(row.required(5)?, FormatStructure::RecordVector, 1)?;
        }
        14 => {
            require_vector_count(row.required(3)?, FormatStructure::OrdinalVector, 1)?;
        }
        19 => {
            require_vector_count(row.required(6)?, FormatStructure::OrdinalVector, 1)?;
        }
        21 => {
            let edge_count =
                require_vector_count(row.required(3)?, FormatStructure::OrdinalVector, 1)?;
            let transition_count = vector_count(row.required(4)?, FormatStructure::RecordVector)?;
            if transition_count != edge_count - 1 {
                return Err(row_binding_mismatch());
            }
        }
        _ => {}
    }
    Ok(())
}

fn require_vector_count(
    field: FieldRef<'_>,
    structure: FormatStructure,
    minimum: u32,
) -> Result<u32, FormatError> {
    let count = vector_count(field, structure)?;
    if count < minimum {
        return Err(row_binding_mismatch());
    }
    Ok(count)
}

fn vector_count(field: FieldRef<'_>, structure: FormatStructure) -> Result<u32, FormatError> {
    read_u32(field.value, 0, structure)
}

fn validate_identity_fields(
    entity: EntityKind,
    field: FieldRef<'_>,
    max_identity_ascii_bytes: u64,
) -> Result<(), FormatError> {
    let required_tags = entity.required_tags();
    let mut index = 0_usize;
    visit_record_rows(field, |row| {
        let actual = validate_identity_field(row, max_identity_ascii_bytes)?;
        if required_tags.get(index) != Some(&actual) {
            return Err(row_binding_mismatch());
        }
        index += 1;
        Ok(())
    })?;
    if index != required_tags.len() {
        return Err(row_binding_mismatch());
    }
    Ok(())
}

fn validate_identity_field(
    row: RowRef<'_>,
    max_identity_ascii_bytes: u64,
) -> Result<FieldTag, FormatError> {
    let tag_field = row.required(1)?;
    let tag_code = tag_field.u16()?;
    let tag =
        FieldTag::from_code(tag_code).ok_or_else(|| unknown(tag_field, u64::from(tag_code)))?;
    let value = row.required(2)?;
    match tag.encoding() {
        FieldEncoding::Ascii => {
            validate_identity_ascii_token(value, max_identity_ascii_bytes)?;
        }
        FieldEncoding::StableId128 if value.value.len() == 16 => {}
        FieldEncoding::StableId128 => {
            return Err(FormatError::LengthMismatch {
                structure: FormatStructure::FieldValue,
                declared: value.value.len() as u64,
                actual: 16,
            });
        }
    }
    Ok(tag)
}

fn validate_kind_id(field: FieldRef<'_>, road_section: bool) -> Result<(), FormatError> {
    validate_ascii_token_grammar(field)?;
    let value = field.value;
    let valid = if road_section {
        value == b"motorLane"
            || value == b"nonMotorLane"
            || value
                .strip_prefix(b"x-lane-")
                .is_some_and(|suffix| !suffix.is_empty())
    } else {
        value == b"sidewalk"
            || value == b"median"
            || value == b"plantingStrip"
            || value == b"facilityStrip"
            || value == b"shoulder"
            || (value
                .strip_prefix(b"x-")
                .is_some_and(|suffix| !suffix.is_empty())
                && !value.starts_with(b"x-lane-"))
    };
    if valid {
        Ok(())
    } else {
        Err(noncanonical(field))
    }
}

fn validate_parking_space(row: RowRef<'_>) -> Result<(), FormatError> {
    let max_progress = MAX_LANE_EDGE_LENGTH_MM - PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM;
    require_u32_inclusive(
        row.required(5)?,
        PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM,
        max_progress,
    )?;
    require_u32_inclusive(
        row.required(7)?,
        PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM,
        max_progress,
    )?;
    require_i32_abs_inclusive(
        row.required(8)?,
        MIN_PARKING_LATERAL_OFFSET_ABS_MM,
        MAX_PARKING_LATERAL_OFFSET_ABS_MM,
    )?;
    require_heading_f32(row.required(9)?)?;
    require_u32_inclusive(
        row.required(10)?,
        MIN_VEHICLE_LENGTH_MM,
        MAX_VEHICLE_LENGTH_MM,
    )?;
    require_u32_inclusive(
        row.required(11)?,
        MIN_VEHICLE_LENGTH_MM,
        MAX_VEHICLE_LENGTH_MM,
    )?;
    Ok(())
}

fn validate_vehicle_profile(row: RowRef<'_>) -> Result<(), FormatError> {
    require_u32_inclusive(
        row.required(4)?,
        MIN_VEHICLE_LENGTH_MM,
        MAX_VEHICLE_LENGTH_MM,
    )?;
    require_u32_inclusive(row.required(5)?, MIN_SPEED_MM_S, MAX_SPEED_MM_S)?;
    require_u32_inclusive(row.required(6)?, 0, MAX_MIN_GAP_MM)?;
    require_f32_positive_at_most(row.required(7)?, MAX_TIME_HEADWAY_SECONDS)?;
    require_f32_inclusive(
        row.required(8)?,
        MIN_ACCEL_METERS_PER_SECOND_SQUARED,
        MAX_ACCEL_METERS_PER_SECOND_SQUARED,
    )?;
    let comfort = require_f32_inclusive(
        row.required(9)?,
        MIN_ACCEL_METERS_PER_SECOND_SQUARED,
        MAX_ACCEL_METERS_PER_SECOND_SQUARED,
    )?;
    let emergency = require_f32_inclusive(
        row.required(10)?,
        MIN_ACCEL_METERS_PER_SECOND_SQUARED,
        MAX_ACCEL_METERS_PER_SECOND_SQUARED,
    )?;
    if emergency < comfort {
        return Err(row_binding_mismatch());
    }
    Ok(())
}

fn validate_geometry_rows(
    points: FieldRef<'_>,
    segments: Option<FieldRef<'_>>,
) -> Result<(), FormatError> {
    let point_count = visit_record_rows(points, |point| {
        for tag in 1..=3 {
            let field = point.required(tag)?;
            if !(-16_384.0..=16_384.0).contains(&field.f32()?) {
                return Err(noncanonical(field));
            }
        }
        Ok(())
    })?;
    if point_count < 2 {
        return Err(row_binding_mismatch());
    }
    if let Some(segments) = segments {
        let segment_count = visit_record_rows(segments, |segment| {
            require_f32_greater(segment.required(1)?, 0.1)?;
            require_f32_greater(segment.required(2)?, 0.1)
        })?;
        if segment_count != point_count - 1 {
            return Err(row_binding_mismatch());
        }
    }
    Ok(())
}

fn validate_direction_profile_applies(
    field: FieldRef<'_>,
    bindings: &DirectBindings,
) -> Result<(), FormatError> {
    let applies = require_u8_range(field, 0, 1)?;
    let direction_profile = bindings
        .lfca_direction_profile
        .ok_or_else(row_binding_mismatch)?;
    if applies != 0 && direction_profile == 0 {
        return Err(row_binding_mismatch());
    }
    Ok(())
}

fn validate_lfsm_row(
    section: u16,
    table: u16,
    row: RowRef<'_>,
    max_identity_ascii_bytes: u64,
    contract_format: u16,
) -> Result<(), FormatError> {
    match (section, table) {
        (1, 1) => {
            require_exact_u16(row.required(1)?, 1)?;
            require_exact_u16(row.required(3)?, contract_format)?;
            require_exact_u16(row.required(7)?, 1)?;
            require_u64_greater(row.required(5)?, 0)?;
            validate_compiler_build_id(row.required(6)?)?;
        }
        (2, 1) => {
            validate_identity_ascii_token(row.required(2)?, max_identity_ascii_bytes)?;
            let language_field = row.required(3)?;
            let language = language_field.u16()?;
            if !matches!(language, 1 | 3) {
                return Err(unknown(language_field, u64::from(language)));
            }
            require_exact_u32(row.required(5)?, 1)?;
            let frontend = row.required(6)?;
            let expected_frontend = if language == 1 { 2 } else { 1 };
            require_exact_u32(frontend, expected_frontend)?;
            visit_record_rows(row.required(12)?, |import| {
                validate_identity_ascii_token(import.required(1)?, max_identity_ascii_bytes)
            })?;
        }
        (2, 2) => {
            validate_ascii_token_grammar(row.required(3)?)?;
            require_u32_greater(row.required(5)?, 0)?;
        }
        (2, 3) => validate_source_location(row, max_identity_ascii_bytes)?,
        (3, 1) => {
            require_entity_kind(row.required(1)?)?;
            if read_u32(row.required(5)?.value, 0, FormatStructure::OrdinalVector)? != 0 {
                return Err(row_binding_mismatch());
            }
        }
        (4, 1) => validate_owner_local_source(row)?,
        (4, 2) => {
            require_exact_u16(row.required(1)?, EntityKind::CanonicalFrame.code())?;
            let role = row.required(3)?;
            if !matches!(role.u8()?, 28 | 29) {
                return Err(unknown(role, u64::from(role.u8()?)));
            }
            if row.required(5)?.u32()? >= row.required(6)?.u32()? {
                return Err(row_binding_mismatch());
            }
        }
        (5, 1) => {
            let owner = require_entity_kind(row.required(1)?)?;
            let role_field = row.required(3)?;
            let role = role_field.u8()?;
            let expected_owner = match role {
                9 => EntityKind::Junction,
                14..=16 => EntityKind::StaticRoute,
                _ => return Err(unknown(role_field, u64::from(role))),
            };
            if owner != expected_owner {
                return Err(row_binding_mismatch());
            }
            require_exact_u16(row.required(5)?, 1)?;
            require_exact_u16(row.required(6)?, contract_format)?;
        }
        _ => {}
    }
    Ok(())
}

fn validate_owner_local_source(row: RowRef<'_>) -> Result<(), FormatError> {
    let owner = require_entity_kind(row.required(1)?)?;
    let role_field = row.required(3)?;
    let role = role_field.u8()?;
    let expected_owner =
        owner_kind_for_source_role(role).ok_or_else(|| unknown(role_field, u64::from(role)))?;
    if owner != expected_owner {
        return Err(row_binding_mismatch());
    }
    Ok(())
}

fn owner_kind_for_source_role(role: u8) -> Option<EntityKind> {
    match role {
        1 => Some(EntityKind::LaneEdge),
        2 => Some(EntityKind::RoadCorridor),
        3 => Some(EntityKind::RoadSection),
        4 => Some(EntityKind::AuthoringLane),
        5 => Some(EntityKind::LaneGroup),
        6 | 9 => Some(EntityKind::Junction),
        7 => Some(EntityKind::Movement),
        8 | 10 | 11 => Some(EntityKind::ManeuverPath),
        12 => Some(EntityKind::StopLine),
        13..=16 => Some(EntityKind::StaticRoute),
        17 | 18 => Some(EntityKind::SignalController),
        19 => Some(EntityKind::SignalPhase),
        20 => Some(EntityKind::ManeuverGate),
        21..=23 => Some(EntityKind::ParkingSpace),
        24 => Some(EntityKind::ParticipantClass),
        25 | 26 => Some(EntityKind::AccessRule),
        27 => Some(EntityKind::VehicleProfile),
        28 | 29 => Some(EntityKind::CanonicalFrame),
        _ => None,
    }
}

fn validate_source_location(
    row: RowRef<'_>,
    max_identity_ascii_bytes: u64,
) -> Result<(), FormatError> {
    let kind = row.required(2)?.u8()?;
    match kind {
        0 => validate_text_location(row),
        1 => validate_road_editing_location(row, max_identity_ascii_bytes),
        _ => Err(unknown(row.required(2)?, u64::from(kind))),
    }
}

fn validate_text_location(row: RowRef<'_>) -> Result<(), FormatError> {
    let start_line = row.required(5)?.u32()?;
    let start_column = row.required(6)?.u32()?;
    let end_line = row.required(7)?.u32()?;
    let end_column = row.required(8)?.u32()?;
    if start_line == 0
        || start_column == 0
        || end_line == 0
        || end_column == 0
        || (start_line, start_column) > (end_line, end_column)
    {
        return Err(noncanonical(row.required(5)?));
    }
    Ok(())
}

fn validate_road_editing_location(
    row: RowRef<'_>,
    max_identity_ascii_bytes: u64,
) -> Result<(), FormatError> {
    for tag in [10, 12, 13, 14, 15] {
        if let Some(field) = row.field(tag) {
            validate_identity_ascii_token(field, max_identity_ascii_bytes)?;
        }
    }

    let subject_field = row.required(9)?;
    let subject = subject_field.u8()?;
    let property = row.field(20);
    match subject {
        0 => {
            forbid_fields(row, 10..=20)?;
        }
        1 => {
            require_fields(row, &[10, 15])?;
            forbid_fields(row, 11..=14)?;
            forbid_fields(row, 16..=19)?;
            if let Some(property) = property {
                validate_property_path(property, 7)?;
            }
        }
        2 => {
            require_fields(row, &[10, 11, 15])?;
            forbid_fields(row, 16..=19)?;
            let entity = require_entity_kind(row.required(11)?)?;
            validate_address_depth(row, entity)?;
            if let Some(property) = property {
                validate_property_path(property, road_table_for_entity(entity))?;
            }
        }
        3 => validate_owner_local_location(row, property)?,
        _ => return Err(unknown(subject_field, u64::from(subject))),
    }
    Ok(())
}

fn validate_owner_local_location(
    row: RowRef<'_>,
    property: Option<FieldRef<'_>>,
) -> Result<(), FormatError> {
    require_fields(row, &[16, 17, 18, 19, 20])?;
    let owner_kind_field = row.required(16)?;
    let owner_kind = require_u8_range(owner_kind_field, 0, 1)?;
    let relation_field = row.required(17)?;
    let relation = require_u8_range(relation_field, 0, 12)?;
    let occurrence = require_u8_range(row.required(18)?, 0, 1)?;
    let (expected_owner, expected_occurrence, root) = road_relation_shape(relation);

    match owner_kind {
        0 => {
            forbid_fields(row, 10..=15)?;
            if relation != 0 || expected_owner.is_some() {
                return Err(row_binding_mismatch());
            }
        }
        1 => {
            require_fields(row, &[10, 15])?;
            let actual_owner = if let Some(kind) = row.field(11) {
                let entity = require_entity_kind(kind)?;
                validate_address_depth(row, entity)?;
                Some(entity)
            } else {
                forbid_fields(row, 12..=14)?;
                None
            };
            let owner_matches = if relation == 1 {
                matches!(actual_owner, None | Some(EntityKind::LaneEdge))
            } else {
                relation != 0 && actual_owner == expected_owner
            };
            if !owner_matches {
                return Err(row_binding_mismatch());
            }
        }
        _ => return Err(unknown(owner_kind_field, u64::from(owner_kind))),
    }
    if occurrence != expected_occurrence {
        return Err(row_binding_mismatch());
    }
    validate_property_path(property.ok_or_else(row_binding_mismatch)?, root)
}

fn road_relation_shape(relation: u8) -> (Option<EntityKind>, u8, u16) {
    match relation {
        0 => (None, 1, 1),
        1 => (None, 0, 5), // LaneEdge 是下方允许的第二种 owner。
        2 => (Some(EntityKind::RoadCorridor), 0, 9),
        3 => (Some(EntityKind::RoadSection), 0, 10),
        4 => (Some(EntityKind::LaneEdge), 1, 12),
        5 | 6 => (Some(EntityKind::Junction), 1, 13),
        7 => (Some(EntityKind::ManeuverPath), 0, 15),
        8 => (Some(EntityKind::SignalController), 1, 20),
        9 => (Some(EntityKind::SignalController), 0, 20),
        10 => (Some(EntityKind::SignalPhase), 1, 22),
        11 => (Some(EntityKind::AccessRule), 1, 31),
        12 => (Some(EntityKind::StaticRoute), 0, 34),
        _ => (None, 0, 0),
    }
}

fn validate_address_depth(row: RowRef<'_>, entity: EntityKind) -> Result<(), FormatError> {
    let depth = match entity {
        EntityKind::RoadSection
        | EntityKind::Movement
        | EntityKind::FacilityBand
        | EntityKind::SignalPhase => 1,
        EntityKind::AuthoringLane | EntityKind::ManeuverPath | EntityKind::LaneGroup => 2,
        EntityKind::ManeuverGate | EntityKind::WaitingZone => 3,
        _ => 0,
    };
    for index in 0..3 {
        if row.has(12 + index) != (usize::from(index) < depth) {
            return Err(row_binding_mismatch());
        }
    }
    Ok(())
}

fn road_table_for_entity(entity: EntityKind) -> u16 {
    match entity {
        EntityKind::RoadCorridor => 9,
        EntityKind::RoadSection => 10,
        EntityKind::AuthoringLane => 11,
        EntityKind::LaneEdge => 12,
        EntityKind::Junction => 13,
        EntityKind::Movement => 14,
        EntityKind::ManeuverPath => 15,
        EntityKind::ManeuverGate => 16,
        EntityKind::WaitingZone => 17,
        EntityKind::StopLine => 18,
        EntityKind::SignalGroup => 19,
        EntityKind::SignalController => 20,
        EntityKind::SignalPhase => 22,
        EntityKind::ParkingArea => 23,
        EntityKind::ParkingSpace => 26,
        EntityKind::LaneGroup => 27,
        EntityKind::FacilityBand => 28,
        EntityKind::ParticipantClass => 29,
        EntityKind::AccessRule => 31,
        EntityKind::VehicleProfile => 33,
        EntityKind::StaticRoute => 34,
        EntityKind::CanonicalFrame => 35,
    }
}

fn validate_property_path(field: FieldRef<'_>, root: u16) -> Result<(), FormatError> {
    let mut steps = [None; MAX_PROPERTY_STEPS];
    let count = visit_record_rows(field, |row| {
        let index = steps
            .iter()
            .position(Option::is_none)
            .ok_or_else(row_binding_mismatch)?;
        let step = PropertyStep {
            kind: row.required(1)?.u8()?,
            container: row.required(2)?.u16()?,
            member: row.required(3)?.u16()?,
        };
        validate_property_step(row, step)?;
        steps[index] = Some(step);
        Ok(())
    })?;
    if !(1..=4).contains(&count) {
        return Err(row_binding_mismatch());
    }
    let count = usize::try_from(count).map_err(|_| FormatError::ArithmeticOverflow {
        structure: FormatStructure::RecordVector,
    })?;
    let first = steps[0].ok_or_else(row_binding_mismatch)?;
    if first.kind != 0 || first.container != root || !property_shape_is_complete(&steps[..count]) {
        return Err(row_binding_mismatch());
    }
    Ok(())
}

fn validate_property_step(row: RowRef<'_>, step: PropertyStep) -> Result<(), FormatError> {
    let valid = match step.kind {
        0 => table_field_max(step.container).is_some_and(|max| step.member <= max),
        1 => {
            step.container <= u16::from(u8::MAX)
                && step.member <= u16::from(u8::MAX)
                && struct_member_max(step.container).is_some_and(|max| step.member <= max)
        }
        2 => {
            step.container <= u16::from(u8::MAX)
                && step.member <= u16::from(u8::MAX)
                && step.container == 0
                && matches!(step.member, 1 | 2)
        }
        _ => return Err(unknown(row.required(1)?, u64::from(step.kind))),
    };
    if valid {
        Ok(())
    } else {
        Err(unknown(
            row.required(2)?,
            (u64::from(step.container) << 16) | u64::from(step.member),
        ))
    }
}

fn property_shape_is_complete(steps: &[Option<PropertyStep>]) -> bool {
    let Some(first) = steps.first().copied().flatten() else {
        return false;
    };
    match steps.len() {
        1 => first.kind == 0,
        2 => {
            let Some(second) = steps[1] else {
                return false;
            };
            direct_struct_edge(first, second) || direct_table_edge(first, second)
        }
        3 => {
            let (Some(second), Some(third)) = (steps[1], steps[2]) else {
                return false;
            };
            table_edge_target(first).is_some_and(|target| {
                second.kind == 0 && second.container == target && direct_struct_edge(second, third)
            })
        }
        4 => {
            let (Some(second), Some(third), Some(fourth)) = (steps[1], steps[2], steps[3]) else {
                return false;
            };
            if first
                != (PropertyStep {
                    kind: 0,
                    container: 5,
                    member: 1,
                })
                || second.kind != 2
                || second.container != 0
            {
                return false;
            }
            let target = match second.member {
                1 => 3,
                2 => 4,
                _ => return false,
            };
            third.kind == 0
                && third.container == target
                && (target == 3 && third.member == 0 || target == 4 && third.member <= 2)
                && fourth.kind == 1
                && fourth.container == 2
                && fourth.member <= 2
        }
        _ => false,
    }
}

fn direct_struct_edge(table: PropertyStep, member: PropertyStep) -> bool {
    table.kind == 0
        && member.kind == 1
        && struct_edge_target(table).is_some_and(|target| member.container == target)
}

fn direct_table_edge(table: PropertyStep, member: PropertyStep) -> bool {
    table.kind == 0
        && member.kind == 0
        && table_edge_target(table).is_some_and(|target| member.container == target)
}

fn struct_edge_target(step: PropertyStep) -> Option<u16> {
    match (step.container, step.member) {
        (2, 2 | 3) => Some(0),
        (2, 4) => Some(1),
        (6, 0) | (3, 0) | (4, 0..=2) => Some(2),
        (11, 3) | (28, 2) => Some(3),
        _ => None,
    }
}

fn table_edge_target(step: PropertyStep) -> Option<u16> {
    match (step.container, step.member) {
        (1, 3) => Some(2),
        (7, 2) | (12, 3) => Some(6),
        (9, 7) => Some(8),
        (22, 2) => Some(21),
        (26, 2 | 3) => Some(24),
        (26, 4) => Some(25),
        (31, 5) => Some(30),
        (33, 2) => Some(32),
        _ => None,
    }
}

fn table_field_max(container: u16) -> Option<u16> {
    const MAX: [u16; 36] = [
        26, 3, 5, 0, 2, 2, 1, 3, 1, 8, 4, 6, 4, 3, 4, 5, 6, 5, 2, 1, 4, 1, 4, 1, 1, 3, 5, 2, 4, 2,
        2, 7, 6, 3, 2, 1,
    ];
    MAX.get(usize::from(container)).copied()
}

fn struct_member_max(container: u16) -> Option<u16> {
    [0, 0, 2, 1].get(usize::from(container)).copied()
}

fn validate_lfsd_row(
    section: u16,
    _table: u16,
    row: RowRef<'_>,
    bindings: &mut DirectBindings,
) -> Result<(), FormatError> {
    match section {
        1 => bindings.lfsd_base_kind = Some(validate_diff_bindings(row)?),
        2..=5 => validate_change_row(
            section,
            row,
            bindings.lfsd_base_kind.ok_or_else(row_binding_mismatch)?,
        )?,
        6 => {
            let kind = row.required(1)?.u8()?;
            if !matches!(kind, 0 | 1) {
                return Err(unknown(row.required(1)?, u64::from(kind)));
            }
            let base_kind = bindings.lfsd_base_kind.ok_or_else(row_binding_mismatch)?;
            if matches!(
                (base_kind, kind),
                (SemanticDiffBaseKind::Genesis, 1) | (SemanticDiffBaseKind::Artifact, 0)
            ) {
                return Err(row_binding_mismatch());
            }
            if kind == 1 && row.required(2)?.value == row.required(3)?.value {
                return Err(row_binding_mismatch());
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_diff_bindings(row: RowRef<'_>) -> Result<SemanticDiffBaseKind, FormatError> {
    let base_kind_field = row.required(1)?;
    let base_kind = require_u8_range(base_kind_field, 0, 1)?;
    if base_kind == 0 {
        require_exact_u16(row.required(2)?, 0)?;
        require_all_zero(row.required(3)?)?;
        require_all_zero(row.required(4)?)?;
        require_exact_u64(row.required(5)?, 0)?;
    } else {
        require_exact_u16(row.required(2)?, 1)?;
        require_not_all_zero(row.required(3)?)?;
        require_not_all_zero(row.required(4)?)?;
        require_u64_greater(row.required(5)?, 0)?;
    }
    require_exact_u16(row.required(6)?, 1)?;
    require_not_all_zero(row.required(7)?)?;
    require_not_all_zero(row.required(8)?)?;
    require_u64_greater(row.required(9)?, 0)?;
    Ok(if base_kind == 0 {
        SemanticDiffBaseKind::Genesis
    } else {
        SemanticDiffBaseKind::Artifact
    })
}

fn validate_change_row(
    section: u16,
    row: RowRef<'_>,
    base_kind: SemanticDiffBaseKind,
) -> Result<(), FormatError> {
    let change = row.required(1)?.u8()?;
    let entity = require_entity_kind(row.required(2)?)?;
    if base_kind == SemanticDiffBaseKind::Genesis && (section == 5 || change != 0) {
        return Err(row_binding_mismatch());
    }
    if section == 4 && !matches!(entity, EntityKind::LaneEdge | EntityKind::FacilityBand) {
        return Err(row_binding_mismatch());
    }
    if section == 3 {
        let role = row.required(5)?;
        let role_value = role.u8()?;
        if !matches!(role_value, 1..=18 | 20..=27) {
            return Err(unknown(role, u64::from(role_value)));
        }
        if owner_kind_for_source_role(role_value) != Some(entity) {
            return Err(row_binding_mismatch());
        }
    }
    if matches!(section, 2 | 5) && row.has(6) {
        let tag = row.required(6)?.u16()?;
        if !diff_field_tag_allowed(section, entity, tag) {
            return Err(unknown(row.required(6)?, u64::from(tag)));
        }
    }
    match (section, change) {
        (2, 2) | (5, 0) => {
            if let (Some(before), Some(after)) = (row.field(9), row.field(10))
                && before.value == after.value
            {
                return Err(row_binding_mismatch());
            }
        }
        (3, 2) => {
            if row.required(7)?.u32()? == row.required(8)?.u32()? {
                return Err(row_binding_mismatch());
            }
        }
        (3, 3) => {
            if row.required(9)?.value == row.required(10)?.value {
                return Err(row_binding_mismatch());
            }
        }
        (4, 2) if row.required(9)?.value == row.required(10)?.value => {
            return Err(row_binding_mismatch());
        }
        _ => {}
    }
    Ok(())
}

fn diff_field_tag_allowed(section: u16, entity: EntityKind, tag: u16) -> bool {
    match section {
        2 => match entity {
            EntityKind::RoadCorridor => tag == 3,
            EntityKind::RoadSection | EntityKind::FacilityBand => tag == 4,
            EntityKind::LaneEdge => matches!(tag, 3 | 4),
            EntityKind::ManeuverGate => tag == 4,
            EntityKind::StopLine => tag == 3,
            EntityKind::ParkingSpace => matches!(tag, 5 | 7..=11),
            EntityKind::ParticipantClass => tag == 4,
            EntityKind::VehicleProfile => (4..=10).contains(&tag),
            _ => false,
        },
        5 => match entity {
            EntityKind::ManeuverGate => tag == 6,
            EntityKind::WaitingZone => (4..=6).contains(&tag),
            EntityKind::SignalController => matches!(tag, 3 | 4),
            EntityKind::SignalPhase => matches!(tag, 4 | 5),
            EntityKind::AccessRule => matches!(tag, 5 | 7 | 8),
            _ => false,
        },
        _ => false,
    }
}

fn validate_lfcp_row(
    section: u16,
    row: RowRef<'_>,
    bindings: &mut DirectBindings,
) -> Result<(), FormatError> {
    match section {
        1 => {
            require_exact_u16(row.required(1)?, bindings.contract_format)?;
            require_exact_u16(row.required(2)?, 1)?;
            require_u64_greater(row.required(5)?, 0)?;
            bindings.lfcp_artifact_digest = Some(copy_digest(row.required(4)?)?);
        }
        2 => {
            require_exact_u16(row.required(1)?, bindings.contract_format)?;
            require_u64_greater(row.required(3)?, 0)?;
            validate_compiler_build_id(row.required(4)?)?;
            require_exact_u16(row.required(5)?, 1)?;
            bindings.lfcp_source_map_digest = Some(copy_digest(row.required(2)?)?);
        }
        3 => {
            require_u8_range(row.required(1)?, 0, 2)?;
            validate_object_key(
                row.required(3)?,
                bindings
                    .lfcp_artifact_digest
                    .ok_or_else(row_binding_mismatch)?,
            )?;
            validate_object_key(
                row.required(4)?,
                bindings
                    .lfcp_source_map_digest
                    .ok_or_else(row_binding_mismatch)?,
            )?;
        }
        _ => {}
    }
    Ok(())
}

fn validate_object_key(field: FieldRef<'_>, digest: [u8; 32]) -> Result<(), FormatError> {
    if field.value.len() != 71
        || !field.value.starts_with(b"sha256/")
        || field.value[7..]
            .iter()
            .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(byte))
    {
        return Err(noncanonical(field));
    }
    for (index, byte) in digest.into_iter().enumerate() {
        let high = hex_lower(byte >> 4);
        let low = hex_lower(byte & 0x0f);
        if field.value[7 + index * 2] != high || field.value[8 + index * 2] != low {
            return Err(FormatError::BindingMismatch {
                structure: FormatStructure::FieldValue,
            });
        }
    }
    Ok(())
}

const fn hex_lower(nibble: u8) -> u8 {
    match nibble {
        0..=9 => b'0' + nibble,
        _ => b'a' + nibble - 10,
    }
}

fn copy_digest(field: FieldRef<'_>) -> Result<[u8; 32], FormatError> {
    field
        .value
        .try_into()
        .map_err(|_| FormatError::LengthMismatch {
            structure: FormatStructure::FieldValue,
            declared: field.value.len() as u64,
            actual: 32,
        })
}

fn require_entity_kind(field: FieldRef<'_>) -> Result<EntityKind, FormatError> {
    let code = field.u16()?;
    EntityKind::from_code(code).ok_or_else(|| unknown(field, u64::from(code)))
}

fn validate_identity_ascii_token(field: FieldRef<'_>, max_bytes: u64) -> Result<(), FormatError> {
    let value = field.value;
    if value.len() as u64 > max_bytes {
        return Err(FormatError::LimitExceeded {
            dimension: LimitDimension::IdentityAsciiBytes,
            actual: value.len() as u64,
            limit: max_bytes,
        });
    }
    validate_ascii_token_grammar(field)
}

fn validate_ascii_token_grammar(field: FieldRef<'_>) -> Result<(), FormatError> {
    let value = field.value;
    if value.is_empty() || !value.is_ascii() {
        return Err(noncanonical(field));
    }
    if !value[0].is_ascii_alphanumeric()
        || value.iter().copied().any(|byte| {
            !byte.is_ascii_alphanumeric() && !matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
        })
    {
        return Err(noncanonical(field));
    }
    Ok(())
}

fn validate_compiler_build_id(field: FieldRef<'_>) -> Result<(), FormatError> {
    let value = field.value;
    if value.is_empty()
        || value.len() > MAX_COMPILER_BUILD_ID_BYTES
        || !value[0].is_ascii_alphanumeric()
        || value.iter().copied().skip(1).any(|byte| {
            !byte.is_ascii_alphanumeric() && !matches!(byte, b'.' | b'_' | b'+' | b'@' | b'-')
        })
    {
        return Err(noncanonical(field));
    }
    Ok(())
}

fn require_scalar_count(
    field: FieldRef<'_>,
    minimum: usize,
    maximum: usize,
) -> Result<(), FormatError> {
    let count = str::from_utf8(field.value)
        .map_err(|_| noncanonical(field))?
        .chars()
        .count();
    if (minimum..=maximum).contains(&count) {
        Ok(())
    } else {
        Err(noncanonical(field))
    }
}

fn require_fields(row: RowRef<'_>, tags: &[u16]) -> Result<(), FormatError> {
    if tags.iter().copied().all(|tag| row.has(tag)) {
        Ok(())
    } else {
        Err(row_binding_mismatch())
    }
}

fn forbid_fields(row: RowRef<'_>, tags: impl Iterator<Item = u16>) -> Result<(), FormatError> {
    if tags.into_iter().any(|tag| row.has(tag)) {
        Err(row_binding_mismatch())
    } else {
        Ok(())
    }
}

fn require_u32_inclusive(field: FieldRef<'_>, min: u32, max: u32) -> Result<u32, FormatError> {
    let value = field.u32()?;
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(noncanonical(field))
    }
}

fn require_i32_abs_inclusive(
    field: FieldRef<'_>,
    min_abs: u32,
    max_abs: u32,
) -> Result<i32, FormatError> {
    let value = field.i32()?;
    let abs = value.unsigned_abs();
    if (min_abs..=max_abs).contains(&abs) {
        Ok(value)
    } else {
        Err(noncanonical(field))
    }
}

fn require_f32_inclusive(field: FieldRef<'_>, min: f32, max: f32) -> Result<f32, FormatError> {
    let value = field.f32()?;
    if value.is_finite() && (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(noncanonical(field))
    }
}

fn require_f32_positive_at_most(field: FieldRef<'_>, max: f32) -> Result<f32, FormatError> {
    let value = field.f32()?;
    if value.is_finite() && value > 0.0 && value <= max {
        Ok(value)
    } else {
        Err(noncanonical(field))
    }
}

fn require_heading_f32(field: FieldRef<'_>) -> Result<f32, FormatError> {
    let bits = field.u32()?;
    if bits == HEADING_PLUS_PI_F32_BITS {
        return Err(noncanonical(field));
    }
    let value = f32::from_bits(bits);
    let min = f32::from_bits(HEADING_MINUS_PI_F32_BITS);
    let max = f32::from_bits(HEADING_PLUS_PI_F32_BITS);
    if value.is_finite() && value >= min && value < max {
        Ok(value)
    } else {
        Err(noncanonical(field))
    }
}

fn require_exact_u16(field: FieldRef<'_>, expected: u16) -> Result<(), FormatError> {
    if field.u16()? == expected {
        Ok(())
    } else {
        Err(noncanonical(field))
    }
}

fn require_exact_u32(field: FieldRef<'_>, expected: u32) -> Result<(), FormatError> {
    if field.u32()? == expected {
        Ok(())
    } else {
        Err(noncanonical(field))
    }
}

fn require_exact_u64(field: FieldRef<'_>, expected: u64) -> Result<(), FormatError> {
    if field.u64()? == expected {
        Ok(())
    } else {
        Err(noncanonical(field))
    }
}

fn require_u8_range(field: FieldRef<'_>, minimum: u8, maximum: u8) -> Result<u8, FormatError> {
    let value = field.u8()?;
    if (minimum..=maximum).contains(&value) {
        Ok(value)
    } else {
        Err(unknown(field, u64::from(value)))
    }
}

fn require_u32_greater(field: FieldRef<'_>, lower: u32) -> Result<(), FormatError> {
    if field.u32()? > lower {
        Ok(())
    } else {
        Err(noncanonical(field))
    }
}

fn require_u64_greater(field: FieldRef<'_>, lower: u64) -> Result<(), FormatError> {
    if field.u64()? > lower {
        Ok(())
    } else {
        Err(noncanonical(field))
    }
}

fn require_f32_greater(field: FieldRef<'_>, lower: f32) -> Result<(), FormatError> {
    if field.f32()? > lower {
        Ok(())
    } else {
        Err(noncanonical(field))
    }
}

fn require_all_zero(field: FieldRef<'_>) -> Result<(), FormatError> {
    if field.value.iter().all(|byte| *byte == 0) {
        Ok(())
    } else {
        Err(noncanonical(field))
    }
}

fn require_not_all_zero(field: FieldRef<'_>) -> Result<(), FormatError> {
    if field.value.iter().any(|byte| *byte != 0) {
        Ok(())
    } else {
        Err(noncanonical(field))
    }
}

fn noncanonical(field: FieldRef<'_>) -> FormatError {
    FormatError::NonCanonicalValue {
        structure: FormatStructure::FieldValue,
        offset: field.value_offset,
    }
}

fn unknown(field: FieldRef<'_>, code: u64) -> FormatError {
    let _ = field;
    FormatError::UnknownKind {
        structure: FormatStructure::FieldValue,
        code,
    }
}

const fn row_binding_mismatch() -> FormatError {
    FormatError::BindingMismatch {
        structure: FormatStructure::RowFields,
    }
}

const fn table_binding_mismatch() -> FormatError {
    FormatError::BindingMismatch {
        structure: FormatStructure::TableRows,
    }
}

#[cfg(test)]
mod tests {
    use std::boxed::Box;
    use std::vec;
    use std::vec::Vec;

    use laneflow_static_contract::{
        OBJECT_PREAMBLE_BYTE_LENGTH, PortableFieldPresence, PortableFieldSchema, PortableFieldType,
        PortableRowCardinality, PortableRowSchema, SECTION_DIRECTORY_ENTRY_BYTE_LENGTH,
        SECTION_FORMAT_VERSION, portable_object_schema,
    };

    use crate::preflight_object_registry;

    use super::*;
    use crate::{FormatErrorClass, FormatLimitConfig};

    fn field_bytes(tag: u16, field_type: PortableFieldType, value: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&tag.to_le_bytes());
        bytes.push(field_type as u8);
        bytes.push(0);
        bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
        bytes.extend_from_slice(value);
        bytes
    }

    fn row_bytes(fields: &[Vec<u8>]) -> Vec<u8> {
        let length = ROW_HEADER_BYTES as usize + fields.iter().map(Vec::len).sum::<usize>();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(length as u64).to_le_bytes());
        bytes.extend_from_slice(&(fields.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        for field in fields {
            bytes.extend_from_slice(field);
        }
        bytes
    }

    fn record_value(rows: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        for row in rows {
            bytes.extend_from_slice(row);
        }
        bytes
    }

    fn ordinal_value(items: &[u32]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(items.len() as u32).to_le_bytes());
        for item in items {
            bytes.extend_from_slice(&item.to_le_bytes());
        }
        bytes
    }

    fn property_value(steps: &[(u8, u16, u16)]) -> Vec<u8> {
        let rows = steps
            .iter()
            .map(|(kind, container, member)| {
                row_bytes(&[
                    field_bytes(1, PortableFieldType::U8, &[*kind]),
                    field_bytes(2, PortableFieldType::U16, &container.to_le_bytes()),
                    field_bytes(3, PortableFieldType::U16, &member.to_le_bytes()),
                ])
            })
            .collect::<Vec<_>>();
        record_value(&rows)
    }

    fn parse_test_row(bytes: &[u8]) -> RowRef<'_> {
        let (row, end) = parse_row(bytes, 0).unwrap();
        assert_eq!(end, bytes.len() as u64);
        row
    }

    fn default_value(
        kind: PortableObjectKind,
        section: u16,
        table: u16,
        field: &PortableFieldSchema,
    ) -> Vec<u8> {
        match field.field_type {
            PortableFieldType::U8 => vec![0],
            PortableFieldType::U16 => {
                let value: u16 = match (kind, section, table, field.tag) {
                    (PortableObjectKind::CanonicalArtifact, 1, 1, 1 | 5 | 6)
                    | (PortableObjectKind::CanonicalArtifact, 6, 1, 1..=2)
                    | (PortableObjectKind::SourceMap, 1, 1, 3)
                    | (PortableObjectKind::CanonicalPublicationDescriptor, 1, 1, 1)
                    | (PortableObjectKind::CanonicalPublicationDescriptor, 2, 1, 1) => {
                        kind.format_version()
                    }
                    (PortableObjectKind::CanonicalArtifact, 1, 1, 2..=4)
                    | (PortableObjectKind::CanonicalArtifact, 7, 1, 2 | 5)
                    | (PortableObjectKind::SourceMap, 1, 1, 1 | 7)
                    | (PortableObjectKind::SemanticDiff, 1, 1, 6)
                    | (PortableObjectKind::CanonicalPublicationDescriptor, 1, 1, 2)
                    | (PortableObjectKind::CanonicalPublicationDescriptor, 2, 1, 5) => 1,
                    _ => 0,
                };
                value.to_le_bytes().to_vec()
            }
            PortableFieldType::U32 | PortableFieldType::F32 | PortableFieldType::I32 => {
                vec![0; 4]
            }
            PortableFieldType::U64 | PortableFieldType::F64 => {
                let value = match (kind, section, table, field.tag) {
                    (PortableObjectKind::SourceMap, 1, 1, 5)
                    | (PortableObjectKind::SemanticDiff, 1, 1, 9)
                    | (PortableObjectKind::CanonicalPublicationDescriptor, 1, 1, 5)
                    | (PortableObjectKind::CanonicalPublicationDescriptor, 2, 1, 3) => 1_u64,
                    _ => 0,
                };
                value.to_le_bytes().to_vec()
            }
            PortableFieldType::StableId128 => vec![0; 16],
            PortableFieldType::Sha256 => {
                if kind == PortableObjectKind::CanonicalArtifact && section == 7 && field.tag == 4 {
                    PORTABLE_COMPILE_OPTIONS_DIGEST_V1.to_vec()
                } else if kind == PortableObjectKind::SemanticDiff
                    && section == 1
                    && matches!(field.tag, 7 | 8)
                    || kind == PortableObjectKind::CanonicalPublicationDescriptor
                        && matches!((section, field.tag), (1, 4) | (2, 2))
                {
                    vec![0x11; 32]
                } else {
                    vec![0; 32]
                }
            }
            PortableFieldType::Utf8 => match (kind, section, table, field.tag) {
                (PortableObjectKind::CanonicalPublicationDescriptor, 3, 1, 3..=4) => {
                    object_key([0x11; 32])
                }
                _ => b"build-v1".to_vec(),
            },
            PortableFieldType::Bytes => Vec::new(),
            PortableFieldType::OrdinalVectorU32 | PortableFieldType::RecordVector => {
                0_u32.to_le_bytes().to_vec()
            }
        }
    }

    fn schema_row_bytes(
        kind: PortableObjectKind,
        section: u16,
        table: u16,
        schema: &PortableRowSchema,
    ) -> Vec<u8> {
        let fields = schema
            .fields
            .iter()
            .filter(|field| field.presence == PortableFieldPresence::Required)
            .map(|field| {
                field_bytes(
                    field.tag,
                    field.field_type,
                    &default_value(kind, section, table, field),
                )
            })
            .collect::<Vec<_>>();
        row_bytes(&fields)
    }

    fn spatial_configuration_change_row(change_kind: u8) -> Vec<u8> {
        let mut fields = vec![field_bytes(1, PortableFieldType::U8, &[change_kind])];
        if change_kind == 1 {
            fields.push(field_bytes(2, PortableFieldType::Bytes, &[0]));
        }
        fields.push(field_bytes(3, PortableFieldType::Bytes, &[1]));
        row_bytes(&fields)
    }

    fn encoded_value_object(kind: PortableObjectKind) -> Vec<u8> {
        let schema = portable_object_schema(kind);
        let sections = schema
            .sections
            .iter()
            .map(|section| {
                let mut bytes = Vec::new();
                bytes.extend_from_slice(&(section.tables.len() as u32).to_le_bytes());
                for table in section.tables {
                    let rows = if table.cardinality == PortableRowCardinality::ExactlyOne {
                        vec![schema_row_bytes(kind, section.kind, table.kind, table.row)]
                    } else if kind == PortableObjectKind::SemanticDiff
                        && section.kind == 6
                        && table.kind == 1
                    {
                        vec![spatial_configuration_change_row(0)]
                    } else {
                        Vec::new()
                    };
                    let rows_length = rows.iter().map(Vec::len).sum::<usize>();
                    bytes.extend_from_slice(&table.kind.to_le_bytes());
                    bytes.extend_from_slice(&1_u16.to_le_bytes());
                    bytes.extend_from_slice(&(rows.len() as u32).to_le_bytes());
                    bytes.extend_from_slice(&(rows_length as u64).to_le_bytes());
                    for row in rows {
                        bytes.extend_from_slice(&row);
                    }
                }
                bytes
            })
            .collect::<Vec<_>>();
        let total = sections
            .iter()
            .map(Vec::len)
            .try_fold(kind.first_section_offset(), |total, length| {
                total.checked_add(length as u64)
            })
            .unwrap();
        let mut bytes = vec![0_u8; kind.first_section_offset() as usize];
        bytes[0..4].copy_from_slice(&kind.magic());
        bytes[4..6].copy_from_slice(&kind.format_version().to_le_bytes());
        bytes[6..8].copy_from_slice(&OBJECT_PREAMBLE_BYTE_LENGTH.to_le_bytes());
        bytes[12..16].copy_from_slice(&kind.section_count().to_le_bytes());
        bytes[16..24].copy_from_slice(&u64::from(OBJECT_PREAMBLE_BYTE_LENGTH).to_le_bytes());
        bytes[24..32].copy_from_slice(&total.to_le_bytes());
        let mut section_offset = kind.first_section_offset();
        for (ordinal, section) in sections.iter().enumerate() {
            let entry = usize::from(OBJECT_PREAMBLE_BYTE_LENGTH)
                + ordinal * SECTION_DIRECTORY_ENTRY_BYTE_LENGTH as usize;
            bytes[entry..entry + 2]
                .copy_from_slice(&u16::try_from(ordinal + 1).unwrap().to_le_bytes());
            bytes[entry + 2..entry + 4].copy_from_slice(&SECTION_FORMAT_VERSION.to_le_bytes());
            bytes[entry + 8..entry + 16].copy_from_slice(&section_offset.to_le_bytes());
            bytes[entry + 16..entry + 24].copy_from_slice(&(section.len() as u64).to_le_bytes());
            section_offset += section.len() as u64;
            bytes.extend_from_slice(section);
        }
        bytes
    }

    fn object_key(digest: [u8; 32]) -> Vec<u8> {
        let mut key = b"sha256/".to_vec();
        for byte in digest {
            key.push(hex_lower(byte >> 4));
            key.push(hex_lower(byte & 0x0f));
        }
        key
    }

    #[test]
    fn every_object_kind_reaches_the_value_checked_capability() {
        for kind in PortableObjectKind::ALL {
            let bytes = encoded_value_object(kind);
            let checked = preflight_object_values(&bytes, kind, FormatLimits::HARD).unwrap();
            assert_eq!(checked.kind(), kind);
            assert_eq!(checked.bytes(), bytes);
            assert_eq!(checked.registry_view().bytes(), bytes);
        }
    }

    #[test]
    fn registry_capability_preserves_the_callers_limits() {
        let kind = PortableObjectKind::CanonicalArtifact;
        let bytes = encoded_value_object(kind);
        let mut config = FormatLimitConfig::HARD;
        config.max_identity_ascii_bytes = 0;
        let limits = FormatLimits::try_new(config).unwrap();
        let registry = preflight_object_registry(&bytes, kind, limits).unwrap();
        assert_eq!(registry.limits(), limits);
    }

    #[test]
    fn identity_fields_reject_unknown_tags_wrong_encoding_and_invalid_tokens() {
        let valid = row_bytes(&[
            field_bytes(1, PortableFieldType::U16, &5_u16.to_le_bytes()),
            field_bytes(2, PortableFieldType::Bytes, b"edge-a"),
        ]);
        validate_identity_field(parse_test_row(&valid), FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES)
            .unwrap();

        let unknown_tag = row_bytes(&[
            field_bytes(1, PortableFieldType::U16, &23_u16.to_le_bytes()),
            field_bytes(2, PortableFieldType::Bytes, b"edge-a"),
        ]);
        assert_eq!(
            validate_identity_field(
                parse_test_row(&unknown_tag),
                FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES,
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::UnknownKind
        );

        let wrong_stable_id = row_bytes(&[
            field_bytes(1, PortableFieldType::U16, &11_u16.to_le_bytes()),
            field_bytes(2, PortableFieldType::Bytes, &[0x11; 15]),
        ]);
        assert_eq!(
            validate_identity_field(
                parse_test_row(&wrong_stable_id),
                FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES,
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::LengthMismatch
        );

        let invalid_token = row_bytes(&[
            field_bytes(1, PortableFieldType::U16, &5_u16.to_le_bytes()),
            field_bytes(2, PortableFieldType::Bytes, b"-edge"),
        ]);
        assert_eq!(
            validate_identity_field(
                parse_test_row(&invalid_token),
                FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES,
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::NonCanonicalValue
        );

        assert_eq!(
            validate_identity_field(parse_test_row(&valid), 5),
            Err(FormatError::LimitExceeded {
                dimension: LimitDimension::IdentityAsciiBytes,
                actual: 6,
                limit: 5,
            })
        );

        let at_ascii_limit = row_bytes(&[
            field_bytes(1, PortableFieldType::U16, &5_u16.to_le_bytes()),
            field_bytes(2, PortableFieldType::Bytes, &[b'a'; 53]),
        ]);
        validate_identity_field(
            parse_test_row(&at_ascii_limit),
            FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES,
        )
        .unwrap();
        let over_ascii_limit = row_bytes(&[
            field_bytes(1, PortableFieldType::U16, &5_u16.to_le_bytes()),
            field_bytes(2, PortableFieldType::Bytes, &[b'a'; 54]),
        ]);
        assert_eq!(
            validate_identity_field(
                parse_test_row(&over_ascii_limit),
                FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES,
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::LimitExceeded
        );
        assert_eq!(
            validate_identity_field(parse_test_row(&at_ascii_limit), 52),
            Err(FormatError::LimitExceeded {
                dimension: LimitDimension::IdentityAsciiBytes,
                actual: 53,
                limit: 52,
            })
        );

        let identity_field = |tags: &[u16]| {
            let rows = tags
                .iter()
                .map(|tag| {
                    row_bytes(&[
                        field_bytes(1, PortableFieldType::U16, &tag.to_le_bytes()),
                        field_bytes(2, PortableFieldType::Bytes, b"edge-a"),
                    ])
                })
                .collect::<Vec<_>>();
            row_bytes(&[field_bytes(
                4,
                PortableFieldType::RecordVector,
                &record_value(&rows),
            )])
        };
        let exact = identity_field(&[1, 5]);
        validate_identity_fields(
            EntityKind::LaneEdge,
            parse_test_row(&exact).required(4).unwrap(),
            FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES,
        )
        .unwrap();
        for tags in [&[1_u16][..], &[5, 1][..], &[1, 5, 5][..]] {
            let invalid = identity_field(tags);
            assert_eq!(
                validate_identity_fields(
                    EntityKind::LaneEdge,
                    parse_test_row(&invalid).required(4).unwrap(),
                    FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES,
                )
                .unwrap_err()
                .class(),
                FormatErrorClass::BindingMismatch
            );
        }
    }

    #[test]
    fn identity_ascii_limit_is_scoped_to_identity_values() {
        let facility_band = row_bytes(&[field_bytes(4, PortableFieldType::Utf8, b"plantingStrip")]);
        validate_lfca_row(
            3,
            17,
            parse_test_row(&facility_band),
            1,
            &mut DirectBindings::default(),
        )
        .unwrap();

        let source_document = row_bytes(&[
            field_bytes(3, PortableFieldType::Utf8, b"document-key"),
            field_bytes(5, PortableFieldType::U32, &1_u32.to_le_bytes()),
        ]);
        validate_lfsm_row(2, 2, parse_test_row(&source_document), 1, 1).unwrap();

        let identity = row_bytes(&[
            field_bytes(1, PortableFieldType::U16, &5_u16.to_le_bytes()),
            field_bytes(2, PortableFieldType::Bytes, b"edge-a"),
        ]);
        assert_eq!(
            validate_identity_field(parse_test_row(&identity), 1),
            Err(FormatError::LimitExceeded {
                dimension: LimitDimension::IdentityAsciiBytes,
                actual: 6,
                limit: 1,
            })
        );
    }

    #[test]
    fn canonical_entity_vector_cardinalities_fail_closed() {
        let road_section = |lanes: &[u32]| {
            row_bytes(&[
                field_bytes(4, PortableFieldType::Utf8, b"motorLane"),
                field_bytes(
                    5,
                    PortableFieldType::OrdinalVectorU32,
                    &ordinal_value(lanes),
                ),
            ])
        };
        assert_eq!(
            validate_lfca_row(
                3,
                2,
                parse_test_row(&road_section(&[])),
                FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES,
                &mut DirectBindings::default(),
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::BindingMismatch
        );
        validate_lfca_row(
            3,
            2,
            parse_test_row(&road_section(&[0])),
            FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES,
            &mut DirectBindings::default(),
        )
        .unwrap();

        let maneuver_path = |edges: &[u32]| {
            row_bytes(&[field_bytes(
                4,
                PortableFieldType::OrdinalVectorU32,
                &ordinal_value(edges),
            )])
        };
        assert_eq!(
            validate_lfca_row(
                3,
                7,
                parse_test_row(&maneuver_path(&[0])),
                FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES,
                &mut DirectBindings::default(),
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::BindingMismatch
        );
        validate_lfca_row(
            3,
            7,
            parse_test_row(&maneuver_path(&[0, 1])),
            FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES,
            &mut DirectBindings::default(),
        )
        .unwrap();

        let static_route = |edges: &[u32], transitions: &[Vec<u8>]| {
            row_bytes(&[
                field_bytes(
                    3,
                    PortableFieldType::OrdinalVectorU32,
                    &ordinal_value(edges),
                ),
                field_bytes(
                    4,
                    PortableFieldType::RecordVector,
                    &record_value(transitions),
                ),
            ])
        };
        assert_eq!(
            validate_lfca_row(
                3,
                21,
                parse_test_row(&static_route(&[0, 1], &[])),
                FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES,
                &mut DirectBindings::default(),
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::BindingMismatch
        );
        validate_lfca_row(
            3,
            21,
            parse_test_row(&static_route(&[0, 1], &[row_bytes(&[])])),
            FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES,
            &mut DirectBindings::default(),
        )
        .unwrap();
    }

    #[test]
    fn direct_versions_build_ids_and_profile_presence_fail_closed() {
        let valid_build = row_bytes(&[field_bytes(
            1,
            PortableFieldType::Utf8,
            b"compiler.v1+ci@main",
        )]);
        validate_compiler_build_id(parse_test_row(&valid_build).required(1).unwrap()).unwrap();
        for invalid in [b"-compiler".as_slice(), b"compiler/path".as_slice()] {
            let row = row_bytes(&[field_bytes(1, PortableFieldType::Utf8, invalid)]);
            assert_eq!(
                validate_compiler_build_id(parse_test_row(&row).required(1).unwrap())
                    .unwrap_err()
                    .class(),
                FormatErrorClass::NonCanonicalValue
            );
        }

        let invalid_publisher = row_bytes(&[field_bytes(1, PortableFieldType::U8, &[3])]);
        assert_eq!(
            validate_lfcp_row(
                3,
                parse_test_row(&invalid_publisher),
                &mut DirectBindings::default()
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::UnknownKind
        );

        let mut bindings = DirectBindings::default();
        let spatial = row_bytes(&[
            field_bytes(1, PortableFieldType::U8, &[1]),
            field_bytes(2, PortableFieldType::U8, &[1]),
        ]);
        validate_lfca_row(
            5,
            1,
            parse_test_row(&spatial),
            FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES,
            &mut bindings,
        )
        .unwrap();
        let provenance = row_bytes(&[
            field_bytes(1, PortableFieldType::Utf8, b"compiler-v1"),
            field_bytes(2, PortableFieldType::U16, &1_u16.to_le_bytes()),
            field_bytes(3, PortableFieldType::Sha256, &[1; 32]),
            field_bytes(
                4,
                PortableFieldType::Sha256,
                &PORTABLE_COMPILE_OPTIONS_DIGEST_V1,
            ),
            field_bytes(5, PortableFieldType::U16, &1_u16.to_le_bytes()),
            field_bytes(6, PortableFieldType::U8, &[0]),
        ]);
        assert_eq!(
            validate_lfca_row(
                7,
                1,
                parse_test_row(&provenance),
                FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES,
                &mut bindings,
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::BindingMismatch
        );
    }

    #[test]
    fn spatial_presence_and_geometry_cardinality_fail_closed() {
        let point = || {
            row_bytes(&[
                field_bytes(1, PortableFieldType::F32, &0.0_f32.to_bits().to_le_bytes()),
                field_bytes(2, PortableFieldType::F32, &0.0_f32.to_bits().to_le_bytes()),
                field_bytes(3, PortableFieldType::F32, &0.0_f32.to_bits().to_le_bytes()),
            ])
        };
        let segment = || {
            row_bytes(&[
                field_bytes(1, PortableFieldType::F32, &0.2_f32.to_bits().to_le_bytes()),
                field_bytes(2, PortableFieldType::F32, &0.2_f32.to_bits().to_le_bytes()),
            ])
        };
        let geometry = |points: &[Vec<u8>], segments: &[Vec<u8>], applies: u8| {
            row_bytes(&[
                field_bytes(3, PortableFieldType::F32, &0.2_f32.to_bits().to_le_bytes()),
                field_bytes(4, PortableFieldType::RecordVector, &record_value(points)),
                field_bytes(5, PortableFieldType::RecordVector, &record_value(segments)),
                field_bytes(6, PortableFieldType::U8, &[applies]),
            ])
        };

        for invalid in [
            geometry(&[], &[], 0),
            geometry(&[point()], &[], 0),
            geometry(&[point(), point()], &[], 0),
        ] {
            let row = parse_test_row(&invalid);
            assert_eq!(
                validate_geometry_rows(row.required(4).unwrap(), Some(row.required(5).unwrap()))
                    .unwrap_err()
                    .class(),
                FormatErrorClass::BindingMismatch
            );
        }
        let valid = geometry(&[point(), point()], &[segment()], 0);
        let valid = parse_test_row(&valid);
        validate_geometry_rows(valid.required(4).unwrap(), Some(valid.required(5).unwrap()))
            .unwrap();

        let facility = row_bytes(&[field_bytes(
            3,
            PortableFieldType::RecordVector,
            &record_value(&[point()]),
        )]);
        assert_eq!(
            validate_geometry_rows(parse_test_row(&facility).required(3).unwrap(), None)
                .unwrap_err()
                .class(),
            FormatErrorClass::BindingMismatch
        );

        let mut bindings = DirectBindings::default();
        let spatial = row_bytes(&[
            field_bytes(1, PortableFieldType::U8, &[1]),
            field_bytes(2, PortableFieldType::U8, &[0]),
        ]);
        validate_lfca_row(
            5,
            1,
            parse_test_row(&spatial),
            FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES,
            &mut bindings,
        )
        .unwrap();
        assert_eq!(
            validate_lfca_row(
                5,
                2,
                parse_test_row(&geometry(&[point(), point()], &[segment()], 1)),
                FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES,
                &mut bindings,
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::BindingMismatch
        );
        assert_eq!(
            validate_object_bindings(PortableObjectKind::CanonicalArtifact, &bindings)
                .unwrap_err()
                .class(),
            FormatErrorClass::BindingMismatch
        );
        record_table_bindings(
            PortableObjectKind::CanonicalArtifact,
            3,
            22,
            1,
            &mut bindings,
        )
        .unwrap();
        validate_object_bindings(PortableObjectKind::CanonicalArtifact, &bindings).unwrap();

        let mut headless = DirectBindings {
            lfca_spatial_present: Some(0),
            lfca_direction_profile: Some(0),
            lfca_has_facility_band_geometry: true,
            ..DirectBindings::default()
        };
        assert_eq!(
            validate_object_bindings(PortableObjectKind::CanonicalArtifact, &headless)
                .unwrap_err()
                .class(),
            FormatErrorClass::BindingMismatch
        );
        headless.lfca_has_facility_band_geometry = false;
        validate_object_bindings(PortableObjectKind::CanonicalArtifact, &headless).unwrap();
    }

    #[test]
    fn source_location_closes_subject_address_depth_and_property_path() {
        let valid_property = property_value(&[(0, 12, 2)]);
        let valid = row_bytes(&[
            field_bytes(1, PortableFieldType::U32, &0_u32.to_le_bytes()),
            field_bytes(2, PortableFieldType::U8, &[1]),
            field_bytes(3, PortableFieldType::U32, &0_u32.to_le_bytes()),
            field_bytes(4, PortableFieldType::U32, &0_u32.to_le_bytes()),
            field_bytes(9, PortableFieldType::U8, &[2]),
            field_bytes(10, PortableFieldType::Utf8, b"city/main"),
            field_bytes(
                11,
                PortableFieldType::U16,
                &EntityKind::LaneEdge.code().to_le_bytes(),
            ),
            field_bytes(15, PortableFieldType::Utf8, b"edge-a"),
            field_bytes(20, PortableFieldType::RecordVector, &valid_property),
        ]);
        validate_lfsm_row(
            2,
            3,
            parse_test_row(&valid),
            FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES,
            1,
        )
        .unwrap();

        let wrong_depth = row_bytes(&[
            field_bytes(1, PortableFieldType::U32, &0_u32.to_le_bytes()),
            field_bytes(2, PortableFieldType::U8, &[1]),
            field_bytes(3, PortableFieldType::U32, &0_u32.to_le_bytes()),
            field_bytes(4, PortableFieldType::U32, &0_u32.to_le_bytes()),
            field_bytes(9, PortableFieldType::U8, &[2]),
            field_bytes(10, PortableFieldType::Utf8, b"city/main"),
            field_bytes(
                11,
                PortableFieldType::U16,
                &EntityKind::ManeuverGate.code().to_le_bytes(),
            ),
            field_bytes(12, PortableFieldType::Utf8, b"path-a"),
            field_bytes(15, PortableFieldType::Utf8, b"gate-a"),
        ]);
        assert_eq!(
            validate_lfsm_row(
                2,
                3,
                parse_test_row(&wrong_depth),
                FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES,
                1,
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::BindingMismatch
        );

        let wrong_property = property_value(&[(0, 12, 3), (0, 31, 0)]);
        let mut fields = vec![
            field_bytes(1, PortableFieldType::U32, &0_u32.to_le_bytes()),
            field_bytes(2, PortableFieldType::U8, &[1]),
            field_bytes(3, PortableFieldType::U32, &0_u32.to_le_bytes()),
            field_bytes(4, PortableFieldType::U32, &0_u32.to_le_bytes()),
            field_bytes(9, PortableFieldType::U8, &[2]),
            field_bytes(10, PortableFieldType::Utf8, b"city/main"),
            field_bytes(
                11,
                PortableFieldType::U16,
                &EntityKind::LaneEdge.code().to_le_bytes(),
            ),
            field_bytes(15, PortableFieldType::Utf8, b"edge-a"),
            field_bytes(20, PortableFieldType::RecordVector, &wrong_property),
        ];
        fields.sort_by_key(|field| u16::from_le_bytes([field[0], field[1]]));
        let wrong_property_row = row_bytes(&fields);
        assert_eq!(
            validate_lfsm_row(
                2,
                3,
                parse_test_row(&wrong_property_row),
                FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES,
                1,
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::BindingMismatch
        );
    }

    #[test]
    fn owner_local_curve_segment_accepts_only_the_two_registered_owner_shapes() {
        for owner in [None, Some(EntityKind::LaneEdge)] {
            let property = property_value(&[(0, 5, 1)]);
            let mut fields = vec![
                field_bytes(1, PortableFieldType::U32, &0_u32.to_le_bytes()),
                field_bytes(2, PortableFieldType::U8, &[1]),
                field_bytes(3, PortableFieldType::U32, &0_u32.to_le_bytes()),
                field_bytes(4, PortableFieldType::U32, &0_u32.to_le_bytes()),
                field_bytes(9, PortableFieldType::U8, &[3]),
                field_bytes(10, PortableFieldType::Utf8, b"city/main"),
                field_bytes(15, PortableFieldType::Utf8, b"alignment-or-edge"),
                field_bytes(16, PortableFieldType::U8, &[1]),
                field_bytes(17, PortableFieldType::U8, &[1]),
                field_bytes(18, PortableFieldType::U8, &[0]),
                field_bytes(19, PortableFieldType::U32, &0_u32.to_le_bytes()),
                field_bytes(20, PortableFieldType::RecordVector, &property),
            ];
            if let Some(owner) = owner {
                fields.push(field_bytes(
                    11,
                    PortableFieldType::U16,
                    &owner.code().to_le_bytes(),
                ));
            }
            fields.sort_by_key(|field| u16::from_le_bytes([field[0], field[1]]));
            let bytes = row_bytes(&fields);
            validate_lfsm_row(
                2,
                3,
                parse_test_row(&bytes),
                FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES,
                1,
            )
            .unwrap();
        }

        let property = property_value(&[(0, 5, 1)]);
        let invalid = row_bytes(&[
            field_bytes(1, PortableFieldType::U32, &0_u32.to_le_bytes()),
            field_bytes(2, PortableFieldType::U8, &[1]),
            field_bytes(3, PortableFieldType::U32, &0_u32.to_le_bytes()),
            field_bytes(4, PortableFieldType::U32, &0_u32.to_le_bytes()),
            field_bytes(9, PortableFieldType::U8, &[3]),
            field_bytes(10, PortableFieldType::Utf8, b"city/main"),
            field_bytes(
                11,
                PortableFieldType::U16,
                &EntityKind::AccessRule.code().to_le_bytes(),
            ),
            field_bytes(15, PortableFieldType::Utf8, b"rule-a"),
            field_bytes(16, PortableFieldType::U8, &[1]),
            field_bytes(17, PortableFieldType::U8, &[1]),
            field_bytes(18, PortableFieldType::U8, &[0]),
            field_bytes(19, PortableFieldType::U32, &0_u32.to_le_bytes()),
            field_bytes(20, PortableFieldType::RecordVector, &property),
        ]);
        assert_eq!(
            validate_lfsm_row(
                2,
                3,
                parse_test_row(&invalid),
                FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES,
                1,
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::BindingMismatch
        );
    }

    #[test]
    fn owner_local_value_matrix_reaches_sixteen_fields_but_not_seventeen() {
        let property = property_value(&[(0, 15, 0)]);
        let mut fields = vec![
            field_bytes(1, PortableFieldType::U32, &0_u32.to_le_bytes()),
            field_bytes(2, PortableFieldType::U8, &[1]),
            field_bytes(3, PortableFieldType::U32, &0_u32.to_le_bytes()),
            field_bytes(4, PortableFieldType::U32, &0_u32.to_le_bytes()),
            field_bytes(9, PortableFieldType::U8, &[3]),
            field_bytes(10, PortableFieldType::Utf8, b"city/main"),
            field_bytes(
                11,
                PortableFieldType::U16,
                &EntityKind::ManeuverPath.code().to_le_bytes(),
            ),
            field_bytes(12, PortableFieldType::Utf8, b"movement-a"),
            field_bytes(13, PortableFieldType::Utf8, b"path-a"),
            field_bytes(15, PortableFieldType::Utf8, b"edge-a"),
            field_bytes(16, PortableFieldType::U8, &[1]),
            field_bytes(17, PortableFieldType::U8, &[7]),
            field_bytes(18, PortableFieldType::U8, &[0]),
            field_bytes(19, PortableFieldType::U32, &0_u32.to_le_bytes()),
            field_bytes(20, PortableFieldType::RecordVector, &property),
            field_bytes(21, PortableFieldType::Utf8, b"canvas-a"),
        ];
        fields.sort_by_key(|field| u16::from_le_bytes([field[0], field[1]]));
        let exact = row_bytes(&fields);
        assert_eq!(parse_test_row(&exact).field_count, 16);
        validate_lfsm_row(
            2,
            3,
            parse_test_row(&exact),
            FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES,
            1,
        )
        .unwrap();

        fields.push(field_bytes(14, PortableFieldType::Utf8, b"extra-depth"));
        fields.sort_by_key(|field| u16::from_le_bytes([field[0], field[1]]));
        let unreachable_seventeen = row_bytes(&fields);
        assert_eq!(parse_test_row(&unreachable_seventeen).field_count, 17);
        assert_eq!(
            validate_lfsm_row(
                2,
                3,
                parse_test_row(&unreachable_seventeen),
                FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES,
                1,
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::BindingMismatch
        );
    }

    fn property_field(steps: &[(u8, u16, u16)]) -> FieldRef<'_> {
        let value = property_value(steps);
        let row = row_bytes(&[field_bytes(20, PortableFieldType::RecordVector, &value)]);
        let row = Box::leak(row.into_boxed_slice());
        parse_test_row(row).required(20).unwrap()
    }

    #[test]
    fn every_registered_property_path_shape_accepts_and_invalid_compositions_fail_closed() {
        const TABLE_FIELD_MAX: [u16; 36] = [
            26, 3, 5, 0, 2, 2, 1, 3, 1, 8, 4, 6, 4, 3, 4, 5, 6, 5, 2, 1, 4, 1, 4, 1, 1, 3, 5, 2, 4,
            2, 2, 7, 6, 3, 2, 1,
        ];
        const STRUCT_MEMBER_MAX: [u16; 4] = [0, 0, 2, 1];
        const STRUCT_EDGES: &[(u16, u16, u16)] = &[
            (2, 2, 0),
            (2, 3, 0),
            (2, 4, 1),
            (3, 0, 2),
            (4, 0, 2),
            (4, 1, 2),
            (4, 2, 2),
            (6, 0, 2),
            (11, 3, 3),
            (28, 2, 3),
        ];
        const TABLE_EDGES: &[(u16, u16, u16)] = &[
            (1, 3, 2),
            (7, 2, 6),
            (9, 7, 8),
            (12, 3, 6),
            (22, 2, 21),
            (26, 2, 24),
            (26, 3, 24),
            (26, 4, 25),
            (31, 5, 30),
            (33, 2, 32),
        ];

        for (table, max_field) in TABLE_FIELD_MAX.into_iter().enumerate() {
            let table = u16::try_from(table).unwrap();
            for field in 0..=max_field {
                validate_property_path(property_field(&[(0, table, field)]), table).unwrap();
            }
            assert_eq!(
                validate_property_path(property_field(&[(0, table, max_field + 1)]), table)
                    .unwrap_err()
                    .class(),
                FormatErrorClass::UnknownKind
            );
        }

        for (table, field, structure) in STRUCT_EDGES {
            for member in 0..=STRUCT_MEMBER_MAX[usize::from(*structure)] {
                validate_property_path(
                    property_field(&[(0, *table, *field), (1, *structure, member)]),
                    *table,
                )
                .unwrap();
            }
        }

        for (table, field, target) in TABLE_EDGES {
            for target_field in 0..=TABLE_FIELD_MAX[usize::from(*target)] {
                validate_property_path(
                    property_field(&[(0, *table, *field), (0, *target, target_field)]),
                    *table,
                )
                .unwrap();
            }
            for (target_table, target_field, structure) in STRUCT_EDGES
                .iter()
                .filter(|(target_table, _, _)| target_table == target)
            {
                for member in 0..=STRUCT_MEMBER_MAX[usize::from(*structure)] {
                    validate_property_path(
                        property_field(&[
                            (0, *table, *field),
                            (0, *target_table, *target_field),
                            (1, *structure, member),
                        ]),
                        *table,
                    )
                    .unwrap();
                }
            }
        }

        for (variant, table, max_field) in [(1, 3, 0), (2, 4, 2)] {
            for field in 0..=max_field {
                for member in 0..=STRUCT_MEMBER_MAX[2] {
                    validate_property_path(
                        property_field(&[
                            (0, 5, 1),
                            (2, 0, variant),
                            (0, table, field),
                            (1, 2, member),
                        ]),
                        5,
                    )
                    .unwrap();
                }
            }
        }

        for invalid in [
            Vec::new(),
            vec![(0, 12, 3), (0, 31, 5)],
            vec![(0, 5, 1), (2, 0, 1)],
            vec![(0, 5, 1), (2, 0, 1), (0, 3, 0), (1, 2, 0), (1, 2, 1)],
        ] {
            assert_eq!(
                validate_property_path(
                    property_field(&invalid),
                    invalid.first().map_or(0, |step| step.1)
                )
                .unwrap_err()
                .class(),
                FormatErrorClass::BindingMismatch
            );
        }
        assert_eq!(
            validate_property_path(property_field(&[(3, 0, 0)]), 0)
                .unwrap_err()
                .class(),
            FormatErrorClass::UnknownKind
        );
        assert_eq!(
            validate_property_path(property_field(&[(0, 12, 2)]), 31)
                .unwrap_err()
                .class(),
            FormatErrorClass::BindingMismatch
        );
    }

    #[test]
    fn every_owner_local_role_has_one_closed_owner_kind() {
        let expected = [
            EntityKind::LaneEdge,
            EntityKind::RoadCorridor,
            EntityKind::RoadSection,
            EntityKind::AuthoringLane,
            EntityKind::LaneGroup,
            EntityKind::Junction,
            EntityKind::Movement,
            EntityKind::ManeuverPath,
            EntityKind::Junction,
            EntityKind::ManeuverPath,
            EntityKind::ManeuverPath,
            EntityKind::StopLine,
            EntityKind::StaticRoute,
            EntityKind::StaticRoute,
            EntityKind::StaticRoute,
            EntityKind::StaticRoute,
            EntityKind::SignalController,
            EntityKind::SignalController,
            EntityKind::SignalPhase,
            EntityKind::ManeuverGate,
            EntityKind::ParkingSpace,
            EntityKind::ParkingSpace,
            EntityKind::ParkingSpace,
            EntityKind::ParticipantClass,
            EntityKind::AccessRule,
            EntityKind::AccessRule,
            EntityKind::VehicleProfile,
            EntityKind::CanonicalFrame,
            EntityKind::CanonicalFrame,
        ];
        assert_eq!(owner_kind_for_source_role(0), None);
        assert_eq!(owner_kind_for_source_role(30), None);
        for (index, owner) in expected.into_iter().enumerate() {
            let role = u8::try_from(index + 1).unwrap();
            assert_eq!(owner_kind_for_source_role(role), Some(owner));
            let valid = row_bytes(&[
                field_bytes(1, PortableFieldType::U16, &owner.code().to_le_bytes()),
                field_bytes(2, PortableFieldType::StableId128, &[1; 16]),
                field_bytes(3, PortableFieldType::U8, &[role]),
                field_bytes(4, PortableFieldType::U32, &0_u32.to_le_bytes()),
                field_bytes(5, PortableFieldType::U32, &0_u32.to_le_bytes()),
                field_bytes(6, PortableFieldType::OrdinalVectorU32, &0_u32.to_le_bytes()),
            ]);
            validate_owner_local_source(parse_test_row(&valid)).unwrap();

            let wrong_owner = EntityKind::ALL
                .into_iter()
                .find(|candidate| *candidate != owner)
                .unwrap();
            let invalid = row_bytes(&[
                field_bytes(1, PortableFieldType::U16, &wrong_owner.code().to_le_bytes()),
                field_bytes(2, PortableFieldType::StableId128, &[1; 16]),
                field_bytes(3, PortableFieldType::U8, &[role]),
                field_bytes(4, PortableFieldType::U32, &0_u32.to_le_bytes()),
                field_bytes(5, PortableFieldType::U32, &0_u32.to_le_bytes()),
                field_bytes(6, PortableFieldType::OrdinalVectorU32, &0_u32.to_le_bytes()),
            ]);
            assert_eq!(
                validate_owner_local_source(parse_test_row(&invalid))
                    .unwrap_err()
                    .class(),
                FormatErrorClass::BindingMismatch
            );
        }
    }

    #[test]
    fn semantic_diff_direct_bindings_and_change_inequalities_fail_closed() {
        let genesis = row_bytes(&[
            field_bytes(1, PortableFieldType::U8, &[0]),
            field_bytes(2, PortableFieldType::U16, &0_u16.to_le_bytes()),
            field_bytes(3, PortableFieldType::Sha256, &[0; 32]),
            field_bytes(4, PortableFieldType::Sha256, &[0; 32]),
            field_bytes(5, PortableFieldType::U64, &0_u64.to_le_bytes()),
            field_bytes(6, PortableFieldType::U16, &1_u16.to_le_bytes()),
            field_bytes(7, PortableFieldType::Sha256, &[1; 32]),
            field_bytes(8, PortableFieldType::Sha256, &[2; 32]),
            field_bytes(9, PortableFieldType::U64, &1_u64.to_le_bytes()),
        ]);
        assert_eq!(
            validate_diff_bindings(parse_test_row(&genesis)).unwrap(),
            SemanticDiffBaseKind::Genesis
        );

        let target_zero = row_bytes(&[
            field_bytes(1, PortableFieldType::U8, &[0]),
            field_bytes(2, PortableFieldType::U16, &0_u16.to_le_bytes()),
            field_bytes(3, PortableFieldType::Sha256, &[0; 32]),
            field_bytes(4, PortableFieldType::Sha256, &[0; 32]),
            field_bytes(5, PortableFieldType::U64, &0_u64.to_le_bytes()),
            field_bytes(6, PortableFieldType::U16, &1_u16.to_le_bytes()),
            field_bytes(7, PortableFieldType::Sha256, &[0; 32]),
            field_bytes(8, PortableFieldType::Sha256, &[2; 32]),
            field_bytes(9, PortableFieldType::U64, &1_u64.to_le_bytes()),
        ]);
        assert_eq!(
            validate_diff_bindings(parse_test_row(&target_zero))
                .unwrap_err()
                .class(),
            FormatErrorClass::NonCanonicalValue
        );

        let move_same_index = row_bytes(&[
            field_bytes(1, PortableFieldType::U8, &[2]),
            field_bytes(
                2,
                PortableFieldType::U16,
                &EntityKind::LaneEdge.code().to_le_bytes(),
            ),
            field_bytes(3, PortableFieldType::StableId128, &[1; 16]),
            field_bytes(4, PortableFieldType::StableId128, &[2; 16]),
            field_bytes(5, PortableFieldType::U8, &[1]),
            field_bytes(7, PortableFieldType::U32, &3_u32.to_le_bytes()),
            field_bytes(8, PortableFieldType::U32, &3_u32.to_le_bytes()),
        ]);
        assert_eq!(
            validate_change_row(
                3,
                parse_test_row(&move_same_index),
                SemanticDiffBaseKind::Artifact,
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::BindingMismatch
        );

        let invalid_entity_field = row_bytes(&[
            field_bytes(1, PortableFieldType::U8, &[2]),
            field_bytes(
                2,
                PortableFieldType::U16,
                &EntityKind::Junction.code().to_le_bytes(),
            ),
            field_bytes(4, PortableFieldType::StableId128, &[2; 16]),
            field_bytes(6, PortableFieldType::U16, &3_u16.to_le_bytes()),
            field_bytes(9, PortableFieldType::Bytes, &[0]),
            field_bytes(10, PortableFieldType::Bytes, &[1]),
        ]);
        assert_eq!(
            validate_change_row(
                2,
                parse_test_row(&invalid_entity_field),
                SemanticDiffBaseKind::Artifact,
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::UnknownKind
        );
    }

    #[test]
    fn semantic_diff_base_kind_closes_change_and_spatial_shapes() {
        let mut genesis = DirectBindings {
            lfsd_base_kind: Some(SemanticDiffBaseKind::Genesis),
            ..DirectBindings::default()
        };
        assert_eq!(
            record_table_bindings(PortableObjectKind::SemanticDiff, 6, 1, 0, &mut genesis,)
                .unwrap_err()
                .class(),
            FormatErrorClass::BindingMismatch
        );
        record_table_bindings(PortableObjectKind::SemanticDiff, 6, 1, 1, &mut genesis).unwrap();
        assert_eq!(
            record_table_bindings(PortableObjectKind::SemanticDiff, 5, 1, 1, &mut genesis,)
                .unwrap_err()
                .class(),
            FormatErrorClass::BindingMismatch
        );

        let remove = row_bytes(&[
            field_bytes(1, PortableFieldType::U8, &[1]),
            field_bytes(
                2,
                PortableFieldType::U16,
                &EntityKind::LaneEdge.code().to_le_bytes(),
            ),
        ]);
        assert_eq!(
            validate_change_row(2, parse_test_row(&remove), SemanticDiffBaseKind::Genesis,)
                .unwrap_err()
                .class(),
            FormatErrorClass::BindingMismatch
        );

        let modify_spatial = spatial_configuration_change_row(1);
        assert_eq!(
            validate_lfsd_row(6, 1, parse_test_row(&modify_spatial), &mut genesis)
                .unwrap_err()
                .class(),
            FormatErrorClass::BindingMismatch
        );
        let mut artifact = DirectBindings {
            lfsd_base_kind: Some(SemanticDiffBaseKind::Artifact),
            ..DirectBindings::default()
        };
        let initialize_spatial = spatial_configuration_change_row(0);
        assert_eq!(
            validate_lfsd_row(6, 1, parse_test_row(&initialize_spatial), &mut artifact)
                .unwrap_err()
                .class(),
            FormatErrorClass::BindingMismatch
        );
    }

    #[test]
    fn geometry_changes_accept_only_geometry_entity_kinds() {
        for change in 0..=2 {
            let mut fields = vec![
                field_bytes(1, PortableFieldType::U8, &[change]),
                field_bytes(
                    2,
                    PortableFieldType::U16,
                    &EntityKind::Junction.code().to_le_bytes(),
                ),
            ];
            if change == 2 {
                fields.push(field_bytes(9, PortableFieldType::Bytes, &[0]));
                fields.push(field_bytes(10, PortableFieldType::Bytes, &[1]));
            }
            assert_eq!(
                validate_change_row(
                    4,
                    parse_test_row(&row_bytes(&fields)),
                    SemanticDiffBaseKind::Artifact,
                )
                .unwrap_err()
                .class(),
                FormatErrorClass::BindingMismatch
            );
        }

        for entity in [EntityKind::LaneEdge, EntityKind::FacilityBand] {
            let add = row_bytes(&[
                field_bytes(1, PortableFieldType::U8, &[0]),
                field_bytes(2, PortableFieldType::U16, &entity.code().to_le_bytes()),
            ]);
            validate_change_row(4, parse_test_row(&add), SemanticDiffBaseKind::Artifact).unwrap();
        }
    }

    #[test]
    fn publication_object_keys_require_lowercase_syntax_and_bound_digest() {
        let digest = [0xab; 32];
        let mut bindings = DirectBindings {
            lfcp_artifact_digest: Some(digest),
            lfcp_source_map_digest: Some(digest),
            ..DirectBindings::default()
        };
        let valid_key = object_key(digest);
        let valid = row_bytes(&[
            field_bytes(1, PortableFieldType::U8, &[1]),
            field_bytes(2, PortableFieldType::Utf8, b"publisher"),
            field_bytes(3, PortableFieldType::Utf8, &valid_key),
            field_bytes(4, PortableFieldType::Utf8, &valid_key),
        ]);
        validate_lfcp_row(3, parse_test_row(&valid), &mut bindings).unwrap();

        let mut uppercase = valid_key.clone();
        uppercase[7] = b'A';
        let invalid_syntax = row_bytes(&[
            field_bytes(1, PortableFieldType::U8, &[1]),
            field_bytes(2, PortableFieldType::Utf8, b"publisher"),
            field_bytes(3, PortableFieldType::Utf8, &uppercase),
            field_bytes(4, PortableFieldType::Utf8, &valid_key),
        ]);
        assert_eq!(
            validate_lfcp_row(3, parse_test_row(&invalid_syntax), &mut bindings)
                .unwrap_err()
                .class(),
            FormatErrorClass::NonCanonicalValue
        );

        let wrong_key = object_key([0xcd; 32]);
        let invalid_binding = row_bytes(&[
            field_bytes(1, PortableFieldType::U8, &[1]),
            field_bytes(2, PortableFieldType::Utf8, b"publisher"),
            field_bytes(3, PortableFieldType::Utf8, &wrong_key),
            field_bytes(4, PortableFieldType::Utf8, &valid_key),
        ]);
        assert_eq!(
            validate_lfcp_row(3, parse_test_row(&invalid_binding), &mut bindings)
                .unwrap_err()
                .class(),
            FormatErrorClass::BindingMismatch
        );
    }
}
