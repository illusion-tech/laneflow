//! 无分配的 v1 受限精确编码。
//!
//! 编码器只拥有线格式、附录 A registry 与通用规范值检查。编译器语义投影、跨表闭包、
//! 摘要和文件系统发布事务不属于本模块。

use laneflow_static_contract::{
    OBJECT_PREAMBLE_V1_BYTE_LENGTH, PortableFieldPresence, PortableFieldSchema, PortableFieldType,
    PortableObjectKind, PortableRowCardinality, PortableRowSchema, PortableRowShape,
    PortableSectionSchema, PortableTableSchema, SECTION_FORMAT_VERSION_V1, portable_field_mask,
    portable_object_schema,
};

use crate::{FormatError, FormatLimits, FormatStructure, LimitDimension};

const SECTION_HEADER_BYTES: u64 = 4;
const TABLE_HEADER_BYTES: u64 = 16;
const ROW_HEADER_BYTES: u64 = 16;
const FIELD_HEADER_BYTES: u64 = 12;
const TABLE_SCHEMA_VERSION_V1: u16 = 1;

/// 一份完整 v1 对象的借用写入输入。
#[derive(Clone, Copy, Debug)]
pub struct ObjectWriteInputV1<'a> {
    pub kind: PortableObjectKind,
    pub sections: &'a [SectionWriteInputV1<'a>],
}

/// 一个 v1 section 的借用写入输入。
#[derive(Clone, Copy, Debug)]
pub struct SectionWriteInputV1<'a> {
    pub kind: u16,
    pub tables: &'a [TableWriteInputV1<'a>],
}

/// 一张 v1 table 的借用写入输入。
#[derive(Clone, Copy, Debug)]
pub struct TableWriteInputV1<'a> {
    pub kind: u16,
    pub rows: &'a [RowWriteInputV1<'a>],
}

/// 一行 v1 row 的借用写入输入。
#[derive(Clone, Copy, Debug)]
pub struct RowWriteInputV1<'a> {
    pub fields: &'a [FieldWriteInputV1<'a>],
}

/// 一个带显式 registry tag 的 v1 field 写入输入。
#[derive(Clone, Copy, Debug)]
pub struct FieldWriteInputV1<'a> {
    pub tag: u16,
    pub value: FieldWriteValueV1<'a>,
}

/// v1 封闭 field type 的有类型写入值。
#[derive(Clone, Copy, Debug)]
pub enum FieldWriteValueV1<'a> {
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    F32(f32),
    F64(f64),
    StableId128([u8; 16]),
    Sha256([u8; 32]),
    Utf8(&'a str),
    Bytes(&'a [u8]),
    OrdinalVectorU32(&'a [u32]),
    RecordVector(&'a [RowWriteInputV1<'a>]),
    I32(i32),
}

impl FieldWriteValueV1<'_> {
    /// 返回将写入 `FieldV1.fieldType` 的封闭登记类型。
    #[must_use]
    pub const fn field_type(self) -> PortableFieldType {
        match self {
            Self::U8(_) => PortableFieldType::U8,
            Self::U16(_) => PortableFieldType::U16,
            Self::U32(_) => PortableFieldType::U32,
            Self::U64(_) => PortableFieldType::U64,
            Self::F32(_) => PortableFieldType::F32,
            Self::F64(_) => PortableFieldType::F64,
            Self::StableId128(_) => PortableFieldType::StableId128,
            Self::Sha256(_) => PortableFieldType::Sha256,
            Self::Utf8(_) => PortableFieldType::Utf8,
            Self::Bytes(_) => PortableFieldType::Bytes,
            Self::OrdinalVectorU32(_) => PortableFieldType::OrdinalVectorU32,
            Self::RecordVector(_) => PortableFieldType::RecordVector,
            Self::I32(_) => PortableFieldType::I32,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct WriteBudget {
    total_utf8_bytes: u64,
    total_vector_bytes: u64,
}

/// 校验完整输入并返回唯一的 exact object byte length。
///
/// 本函数不分配、不写入，也不执行编译器语义或跨对象绑定验证。
pub fn measure_object_v1(
    input: ObjectWriteInputV1<'_>,
    limits: FormatLimits,
) -> Result<u64, FormatError> {
    let schema = portable_object_schema(input.kind);
    check_exact_count(
        FormatStructure::SectionDirectory,
        input.sections.len(),
        schema.sections.len(),
    )?;

    let mut budget = WriteBudget::default();
    let mut object_length = input.kind.first_section_offset();
    for (ordinal, (section, section_schema)) in
        input.sections.iter().zip(schema.sections).enumerate()
    {
        check_ordered_kind(
            FormatStructure::SectionDirectory,
            section.kind,
            section_schema.kind,
            ordinal,
            schema.sections.len(),
        )?;
        let section_length =
            measure_section(input.kind, *section, section_schema, limits, &mut budget)?;
        check_limit(
            LimitDimension::SectionOrTableBytes,
            section_length,
            limits.config().max_section_or_table_bytes,
        )?;
        object_length = checked_add(
            object_length,
            section_length,
            FormatStructure::ObjectPreamble,
        )?;
        check_limit(
            LimitDimension::ObjectBytes,
            object_length,
            limits.config().max_object_bytes,
        )?;
    }
    Ok(object_length)
}

/// 把完整输入精确编码到调用方提供的缓冲区。
///
/// 编码器先完成与 [`measure_object_v1`] 相同的全对象预检，并在缓冲区长度不精确时直接
/// 失败。任何返回的错误都发生在写入开始前，因此 `output` 保持逐字节不变。成功时整个
/// `output` 恰好是一份无 padding、无尾字节的 v1 对象。
pub fn encode_object_v1(
    input: ObjectWriteInputV1<'_>,
    limits: FormatLimits,
    output: &mut [u8],
) -> Result<(), FormatError> {
    let object_length = measure_object_v1(input, limits)?;
    let output_length = output.len() as u64;
    if output_length != object_length {
        return Err(FormatError::LengthMismatch {
            structure: FormatStructure::ObjectPreamble,
            declared: output_length,
            actual: object_length,
        });
    }

    let mut cursor = WriteCursor::new(output);
    write_object(&mut cursor, input, object_length);
    debug_assert_eq!(cursor.position(), output_length);
    Ok(())
}

fn measure_section(
    object_kind: PortableObjectKind,
    section: SectionWriteInputV1<'_>,
    schema: &'static PortableSectionSchema,
    limits: FormatLimits,
    budget: &mut WriteBudget,
) -> Result<u64, FormatError> {
    check_exact_count(
        FormatStructure::Section,
        section.tables.len(),
        schema.tables.len(),
    )?;
    let mut section_length = SECTION_HEADER_BYTES;
    for (ordinal, (table, table_schema)) in section.tables.iter().zip(schema.tables).enumerate() {
        check_ordered_kind(
            FormatStructure::Section,
            table.kind,
            table_schema.kind,
            ordinal,
            schema.tables.len(),
        )?;
        let is_source_location =
            object_kind == PortableObjectKind::SourceMap && section.kind == 2 && table.kind == 3;
        let table_length = measure_table(*table, table_schema, is_source_location, limits, budget)?;
        check_limit(
            LimitDimension::SectionOrTableBytes,
            table_length,
            limits.config().max_section_or_table_bytes,
        )?;
        section_length = checked_add(section_length, table_length, FormatStructure::Section)?;
        check_limit(
            LimitDimension::SectionOrTableBytes,
            section_length,
            limits.config().max_section_or_table_bytes,
        )?;
    }
    Ok(section_length)
}

fn measure_table(
    table: TableWriteInputV1<'_>,
    schema: &'static PortableTableSchema,
    is_source_location: bool,
    limits: FormatLimits,
    budget: &mut WriteBudget,
) -> Result<u64, FormatError> {
    let row_count = count_u32(table.rows.len(), FormatStructure::TableRows)?;
    check_limit(
        LimitDimension::RowsPerTable,
        u64::from(row_count),
        u64::from(limits.config().max_rows_per_table),
    )?;
    if is_source_location {
        check_limit(
            LimitDimension::SourceLocationRows,
            u64::from(row_count),
            u64::from(limits.max_source_location_rows()),
        )?;
    }
    let cardinality_matches = match schema.cardinality {
        PortableRowCardinality::Any => true,
        PortableRowCardinality::AtMostOne => row_count <= 1,
        PortableRowCardinality::ExactlyOne => row_count == 1,
    };
    if !cardinality_matches {
        return Err(FormatError::BindingMismatch {
            structure: FormatStructure::TableRows,
        });
    }

    let mut table_length = TABLE_HEADER_BYTES;
    for row in table.rows {
        table_length = checked_add(
            table_length,
            measure_row(*row, schema.row, 0, limits, budget)?,
            FormatStructure::Table,
        )?;
        check_limit(
            LimitDimension::SectionOrTableBytes,
            table_length,
            limits.config().max_section_or_table_bytes,
        )?;
    }
    Ok(table_length)
}

fn measure_row(
    row: RowWriteInputV1<'_>,
    schema: &'static PortableRowSchema,
    depth: u8,
    limits: FormatLimits,
    budget: &mut WriteBudget,
) -> Result<u64, FormatError> {
    let field_count = count_u32(row.fields.len(), FormatStructure::RowFields)?;
    check_limit(
        LimitDimension::FieldsPerRow,
        u64::from(field_count),
        u64::from(limits.config().max_fields_per_row),
    )?;

    let mut row_length = ROW_HEADER_BYTES;
    let mut previous_tag = None;
    let mut schema_index = 0_usize;
    let mut seen_fields = 0_u32;
    let mut discriminant = None;
    for field in row.fields {
        if field.tag == 0 {
            return Err(FormatError::UnknownKind {
                structure: FormatStructure::Field,
                code: 0,
            });
        }
        if let Some(previous) = previous_tag
            && previous >= field.tag
        {
            return Err(FormatError::NonCanonicalOrder {
                structure: FormatStructure::RowFields,
                previous: u64::from(previous),
                current: u64::from(field.tag),
            });
        }
        previous_tag = Some(field.tag);

        while schema_index < schema.fields.len() && schema.fields[schema_index].tag < field.tag {
            schema_index += 1;
        }
        let field_schema =
            schema
                .fields
                .get(schema_index)
                .copied()
                .ok_or(FormatError::UnknownKind {
                    structure: FormatStructure::Field,
                    code: u64::from(field.tag),
                })?;
        if field_schema.tag != field.tag {
            return Err(FormatError::UnknownKind {
                structure: FormatStructure::Field,
                code: u64::from(field.tag),
            });
        }
        if field.value.field_type() != field_schema.field_type {
            return Err(FormatError::BindingMismatch {
                structure: FormatStructure::Field,
            });
        }
        schema_index += 1;
        seen_fields |= portable_field_mask(field.tag);
        if let PortableRowShape::DiscriminatedU8 { tag, .. } = schema.shape
            && field.tag == tag
            && let FieldWriteValueV1::U8(value) = field.value
        {
            discriminant = Some(value);
        }

        let value_length = measure_value(field.value, field_schema, depth, limits, budget)?;
        row_length = checked_add(
            row_length,
            checked_add(FIELD_HEADER_BYTES, value_length, FormatStructure::Field)?,
            FormatStructure::Row,
        )?;
    }
    validate_row_shape(schema, seen_fields, discriminant)?;
    Ok(row_length)
}

fn measure_value(
    value: FieldWriteValueV1<'_>,
    schema: PortableFieldSchema,
    depth: u8,
    limits: FormatLimits,
    budget: &mut WriteBudget,
) -> Result<u64, FormatError> {
    let value_length = match value {
        FieldWriteValueV1::U8(_) => 1,
        FieldWriteValueV1::U16(_) => 2,
        FieldWriteValueV1::U32(_) | FieldWriteValueV1::F32(_) | FieldWriteValueV1::I32(_) => 4,
        FieldWriteValueV1::U64(_) | FieldWriteValueV1::F64(_) => 8,
        FieldWriteValueV1::StableId128(_) => 16,
        FieldWriteValueV1::Sha256(_) => 32,
        FieldWriteValueV1::Utf8(value) => {
            let length = value.len() as u64;
            check_limit(
                LimitDimension::Utf8FieldBytes,
                length,
                limits.config().max_utf8_field_bytes,
            )?;
            budget.total_utf8_bytes =
                checked_add(budget.total_utf8_bytes, length, FormatStructure::FieldValue)?;
            check_limit(
                LimitDimension::TotalUtf8Bytes,
                budget.total_utf8_bytes,
                limits.config().max_total_utf8_bytes,
            )?;
            length
        }
        FieldWriteValueV1::Bytes(value) => value.len() as u64,
        FieldWriteValueV1::OrdinalVectorU32(items) => {
            let count = count_u32(items.len(), FormatStructure::OrdinalVector)?;
            check_limit(
                LimitDimension::VectorItems,
                u64::from(count),
                u64::from(limits.config().max_vector_items),
            )?;
            let items_length =
                u64::from(count)
                    .checked_mul(4)
                    .ok_or(FormatError::ArithmeticOverflow {
                        structure: FormatStructure::OrdinalVector,
                    })?;
            checked_add(4, items_length, FormatStructure::OrdinalVector)?
        }
        FieldWriteValueV1::RecordVector(rows) => {
            let next_depth = depth
                .checked_add(1)
                .ok_or(FormatError::ArithmeticOverflow {
                    structure: FormatStructure::RecordVector,
                })?;
            check_limit(
                LimitDimension::RecordVectorDepth,
                u64::from(next_depth),
                u64::from(limits.config().max_record_vector_depth),
            )?;
            let count = count_u32(rows.len(), FormatStructure::RecordVector)?;
            check_limit(
                LimitDimension::VectorItems,
                u64::from(count),
                u64::from(limits.config().max_vector_items),
            )?;
            let nested_schema = schema.nested_row.ok_or(FormatError::BindingMismatch {
                structure: FormatStructure::RecordVector,
            })?;
            let mut length = 4_u64;
            for row in rows {
                length = checked_add(
                    length,
                    measure_row(*row, nested_schema, next_depth, limits, budget)?,
                    FormatStructure::RecordVector,
                )?;
            }
            length
        }
    };

    match value {
        FieldWriteValueV1::F32(value) if !canonical_f32(value) => {
            return Err(noncanonical_value());
        }
        FieldWriteValueV1::F64(value) if !canonical_f64(value) => {
            return Err(noncanonical_value());
        }
        _ => {}
    }

    if matches!(
        value,
        FieldWriteValueV1::OrdinalVectorU32(_) | FieldWriteValueV1::RecordVector(_)
    ) {
        budget.total_vector_bytes = checked_add(
            budget.total_vector_bytes,
            value_length,
            FormatStructure::FieldValue,
        )?;
        check_limit(
            LimitDimension::TotalVectorBytes,
            budget.total_vector_bytes,
            limits.config().max_total_vector_bytes,
        )?;
    }
    Ok(value_length)
}

fn validate_row_shape(
    schema: &PortableRowSchema,
    seen_fields: u32,
    discriminant: Option<u8>,
) -> Result<(), FormatError> {
    match schema.shape {
        PortableRowShape::Uniform => {
            for field in schema.fields {
                if field.presence == PortableFieldPresence::Required
                    && seen_fields & portable_field_mask(field.tag) == 0
                {
                    return Err(FormatError::BindingMismatch {
                        structure: FormatStructure::RowFields,
                    });
                }
            }
        }
        PortableRowShape::DiscriminatedU8 { variants, .. } => {
            let discriminant = discriminant.ok_or(FormatError::BindingMismatch {
                structure: FormatStructure::RowFields,
            })?;
            let variant = variants
                .iter()
                .find(|variant| variant.discriminant == discriminant)
                .ok_or(FormatError::UnknownKind {
                    structure: FormatStructure::FieldValue,
                    code: u64::from(discriminant),
                })?;
            if seen_fields & variant.required_fields != variant.required_fields
                || seen_fields & !variant.allowed_fields != 0
                || (variant.at_least_one_field != 0
                    && seen_fields & variant.at_least_one_field == 0)
            {
                return Err(FormatError::BindingMismatch {
                    structure: FormatStructure::RowFields,
                });
            }
        }
    }
    Ok(())
}

fn canonical_f32(value: f32) -> bool {
    value.is_finite() && value.to_bits() != 0x8000_0000
}

fn canonical_f64(value: f64) -> bool {
    value.is_finite() && value.to_bits() != 0x8000_0000_0000_0000
}

const fn noncanonical_value() -> FormatError {
    FormatError::NonCanonicalValue {
        structure: FormatStructure::FieldValue,
        offset: 0,
    }
}

fn check_exact_count(
    structure: FormatStructure,
    actual: usize,
    expected: usize,
) -> Result<(), FormatError> {
    if actual != expected {
        return Err(FormatError::LengthMismatch {
            structure,
            declared: actual as u64,
            actual: expected as u64,
        });
    }
    Ok(())
}

fn check_ordered_kind(
    structure: FormatStructure,
    actual: u16,
    expected: u16,
    ordinal: usize,
    count: usize,
) -> Result<(), FormatError> {
    if actual == 0 || usize::from(actual) > count {
        return Err(FormatError::UnknownKind {
            structure,
            code: u64::from(actual),
        });
    }
    if actual != expected {
        let expected_from_ordinal = u64::try_from(ordinal)
            .ok()
            .and_then(|value| value.checked_add(1))
            .unwrap_or(u64::from(expected));
        return Err(FormatError::NonCanonicalOrder {
            structure,
            previous: expected_from_ordinal,
            current: u64::from(actual),
        });
    }
    Ok(())
}

fn count_u32(count: usize, structure: FormatStructure) -> Result<u32, FormatError> {
    u32::try_from(count).map_err(|_| FormatError::ArithmeticOverflow { structure })
}

fn checked_add(left: u64, right: u64, structure: FormatStructure) -> Result<u64, FormatError> {
    left.checked_add(right)
        .ok_or(FormatError::ArithmeticOverflow { structure })
}

fn check_limit(dimension: LimitDimension, actual: u64, limit: u64) -> Result<(), FormatError> {
    if actual > limit {
        return Err(FormatError::LimitExceeded {
            dimension,
            actual,
            limit,
        });
    }
    Ok(())
}

fn raw_section_length(section: SectionWriteInputV1<'_>) -> u64 {
    SECTION_HEADER_BYTES
        + section
            .tables
            .iter()
            .copied()
            .map(raw_table_length)
            .sum::<u64>()
}

fn raw_table_length(table: TableWriteInputV1<'_>) -> u64 {
    TABLE_HEADER_BYTES + table.rows.iter().copied().map(raw_row_length).sum::<u64>()
}

fn raw_row_length(row: RowWriteInputV1<'_>) -> u64 {
    ROW_HEADER_BYTES
        + row
            .fields
            .iter()
            .map(|field| FIELD_HEADER_BYTES + raw_value_length(field.value))
            .sum::<u64>()
}

fn raw_value_length(value: FieldWriteValueV1<'_>) -> u64 {
    match value {
        FieldWriteValueV1::U8(_) => 1,
        FieldWriteValueV1::U16(_) => 2,
        FieldWriteValueV1::U32(_) | FieldWriteValueV1::F32(_) | FieldWriteValueV1::I32(_) => 4,
        FieldWriteValueV1::U64(_) | FieldWriteValueV1::F64(_) => 8,
        FieldWriteValueV1::StableId128(_) => 16,
        FieldWriteValueV1::Sha256(_) => 32,
        FieldWriteValueV1::Utf8(value) => value.len() as u64,
        FieldWriteValueV1::Bytes(value) => value.len() as u64,
        FieldWriteValueV1::OrdinalVectorU32(items) => 4 + items.len() as u64 * 4,
        FieldWriteValueV1::RecordVector(rows) => {
            4 + rows.iter().copied().map(raw_row_length).sum::<u64>()
        }
    }
}

fn write_object(cursor: &mut WriteCursor<'_>, input: ObjectWriteInputV1<'_>, object_length: u64) {
    cursor.put(&input.kind.magic());
    cursor.u16(input.kind.format_version());
    cursor.u16(OBJECT_PREAMBLE_V1_BYTE_LENGTH);
    cursor.u32(0);
    cursor.u32(input.kind.section_count());
    cursor.u64(u64::from(OBJECT_PREAMBLE_V1_BYTE_LENGTH));
    cursor.u64(object_length);

    let mut section_offset = input.kind.first_section_offset();
    for section in input.sections {
        let section_length = raw_section_length(*section);
        cursor.u16(section.kind);
        cursor.u16(SECTION_FORMAT_VERSION_V1);
        cursor.u32(0);
        cursor.u64(section_offset);
        cursor.u64(section_length);
        section_offset += section_length;
    }
    for section in input.sections {
        write_section(cursor, *section);
    }
}

fn write_section(cursor: &mut WriteCursor<'_>, section: SectionWriteInputV1<'_>) {
    cursor.u32(section.tables.len() as u32);
    for table in section.tables {
        write_table(cursor, *table);
    }
}

fn write_table(cursor: &mut WriteCursor<'_>, table: TableWriteInputV1<'_>) {
    let table_length = raw_table_length(table);
    cursor.u16(table.kind);
    cursor.u16(TABLE_SCHEMA_VERSION_V1);
    cursor.u32(table.rows.len() as u32);
    cursor.u64(table_length - TABLE_HEADER_BYTES);
    for row in table.rows {
        write_row(cursor, *row);
    }
}

fn write_row(cursor: &mut WriteCursor<'_>, row: RowWriteInputV1<'_>) {
    cursor.u64(raw_row_length(row));
    cursor.u32(row.fields.len() as u32);
    cursor.u32(0);
    for field in row.fields {
        cursor.u16(field.tag);
        cursor.u8(field.value.field_type() as u8);
        cursor.u8(0);
        cursor.u64(raw_value_length(field.value));
        write_value(cursor, field.value);
    }
}

fn write_value(cursor: &mut WriteCursor<'_>, value: FieldWriteValueV1<'_>) {
    match value {
        FieldWriteValueV1::U8(value) => cursor.u8(value),
        FieldWriteValueV1::U16(value) => cursor.u16(value),
        FieldWriteValueV1::U32(value) => cursor.u32(value),
        FieldWriteValueV1::U64(value) => cursor.u64(value),
        FieldWriteValueV1::F32(value) => cursor.u32(value.to_bits()),
        FieldWriteValueV1::F64(value) => cursor.u64(value.to_bits()),
        FieldWriteValueV1::StableId128(value) => cursor.put(&value),
        FieldWriteValueV1::Sha256(value) => cursor.put(&value),
        FieldWriteValueV1::Utf8(value) => cursor.put(value.as_bytes()),
        FieldWriteValueV1::Bytes(value) => cursor.put(value),
        FieldWriteValueV1::OrdinalVectorU32(items) => {
            cursor.u32(items.len() as u32);
            for item in items {
                cursor.u32(*item);
            }
        }
        FieldWriteValueV1::RecordVector(rows) => {
            cursor.u32(rows.len() as u32);
            for row in rows {
                write_row(cursor, *row);
            }
        }
        FieldWriteValueV1::I32(value) => cursor.put(&value.to_le_bytes()),
    }
}

struct WriteCursor<'a> {
    output: &'a mut [u8],
    position: usize,
}

impl<'a> WriteCursor<'a> {
    const fn new(output: &'a mut [u8]) -> Self {
        Self {
            output,
            position: 0,
        }
    }

    fn position(&self) -> u64 {
        self.position as u64
    }

    fn put(&mut self, bytes: &[u8]) {
        let end = self.position + bytes.len();
        self.output[self.position..end].copy_from_slice(bytes);
        self.position = end;
    }

    fn u8(&mut self, value: u8) {
        self.put(&[value]);
    }

    fn u16(&mut self, value: u16) {
        self.put(&value.to_le_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.put(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.put(&value.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use std::{boxed::Box, vec, vec::Vec};

    use laneflow_static_contract::{
        FORMAT_HARD_MAX_OBJECT_BYTES, PortableFieldPresence, PortableRowCardinality,
        PortableRowShape, portable_object_schema,
    };

    use super::*;
    use crate::{
        FormatErrorClass, FormatLimitConfig, RegistryCheckedFieldValue,
        preflight_object_registry_v1, preflight_object_values_v1,
    };

    fn leak<T>(values: Vec<T>) -> &'static [T] {
        Box::leak(values.into_boxed_slice())
    }

    fn selected_row_fields(schema: &PortableRowSchema) -> (u32, Option<(u16, u8)>) {
        match schema.shape {
            PortableRowShape::Uniform => {
                let mut fields = 0_u32;
                for field in schema.fields {
                    if field.presence == PortableFieldPresence::Required {
                        fields |= portable_field_mask(field.tag);
                    }
                }
                (fields, None)
            }
            PortableRowShape::DiscriminatedU8 { tag, variants } => {
                let variant = variants[0];
                let mut fields = variant.required_fields;
                if variant.at_least_one_field != 0 && fields & variant.at_least_one_field == 0 {
                    fields |= 1_u32 << variant.at_least_one_field.trailing_zeros();
                }
                (fields, Some((tag, variant.discriminant)))
            }
        }
    }

    fn default_value(
        field: &PortableFieldSchema,
        discriminant: Option<(u16, u8)>,
        populate_records: bool,
    ) -> FieldWriteValueV1<'static> {
        match field.field_type {
            PortableFieldType::U8 => FieldWriteValueV1::U8(
                discriminant
                    .filter(|(tag, _)| *tag == field.tag)
                    .map_or(0, |(_, value)| value),
            ),
            PortableFieldType::U16 => FieldWriteValueV1::U16(0),
            PortableFieldType::U32 => FieldWriteValueV1::U32(0),
            PortableFieldType::U64 => FieldWriteValueV1::U64(0),
            PortableFieldType::F32 => FieldWriteValueV1::F32(0.0),
            PortableFieldType::F64 => FieldWriteValueV1::F64(0.0),
            PortableFieldType::StableId128 => FieldWriteValueV1::StableId128([0; 16]),
            PortableFieldType::Sha256 => FieldWriteValueV1::Sha256([0; 32]),
            PortableFieldType::Utf8 => FieldWriteValueV1::Utf8(""),
            PortableFieldType::Bytes => FieldWriteValueV1::Bytes(&[]),
            PortableFieldType::OrdinalVectorU32 => FieldWriteValueV1::OrdinalVectorU32(&[]),
            PortableFieldType::RecordVector => {
                if populate_records {
                    let nested = leak(vec![default_row(
                        field.nested_row.expect("registry supplies nested row"),
                        false,
                    )]);
                    FieldWriteValueV1::RecordVector(nested)
                } else {
                    FieldWriteValueV1::RecordVector(&[])
                }
            }
            PortableFieldType::I32 => FieldWriteValueV1::I32(0),
        }
    }

    fn default_row(
        schema: &'static PortableRowSchema,
        populate_records: bool,
    ) -> RowWriteInputV1<'static> {
        let (selected, discriminant) = selected_row_fields(schema);
        let fields = schema
            .fields
            .iter()
            .filter(|field| selected & portable_field_mask(field.tag) != 0)
            .map(|field| FieldWriteInputV1 {
                tag: field.tag,
                value: default_value(field, discriminant, populate_records),
            })
            .collect();
        RowWriteInputV1 {
            fields: leak(fields),
        }
    }

    fn fixture_object(
        kind: PortableObjectKind,
        populate_first_record_vector: bool,
    ) -> ObjectWriteInputV1<'static> {
        let schema = portable_object_schema(kind);
        let mut record_populated = false;
        let sections = schema
            .sections
            .iter()
            .map(|section| {
                let tables = section
                    .tables
                    .iter()
                    .map(|table| {
                        let has_record = table
                            .row
                            .fields
                            .iter()
                            .any(|field| field.field_type == PortableFieldType::RecordVector);
                        let should_populate =
                            populate_first_record_vector && !record_populated && has_record;
                        record_populated |= should_populate;
                        let row_required = table.cardinality == PortableRowCardinality::ExactlyOne;
                        let rows = if row_required || should_populate {
                            leak(vec![default_row(table.row, should_populate)])
                        } else {
                            &[]
                        };
                        TableWriteInputV1 {
                            kind: table.kind,
                            rows,
                        }
                    })
                    .collect();
                SectionWriteInputV1 {
                    kind: section.kind,
                    tables: leak(tables),
                }
            })
            .collect();
        assert!(!populate_first_record_vector || record_populated);
        ObjectWriteInputV1 {
            kind,
            sections: leak(sections),
        }
    }

    fn lfsd_with_entity_add_payload(payload: &'static [u8]) -> ObjectWriteInputV1<'static> {
        let input = fixture_object(PortableObjectKind::SemanticDiff, false);
        let mut sections = input.sections.to_vec();

        let mut binding_tables = sections[0].tables.to_vec();
        let mut binding_rows = binding_tables[0].rows.to_vec();
        let mut binding_fields = binding_rows[0].fields.to_vec();
        for field in &mut binding_fields {
            field.value = match field.tag {
                6 => FieldWriteValueV1::U16(1),
                7 => FieldWriteValueV1::Sha256([1; 32]),
                8 => FieldWriteValueV1::Sha256([2; 32]),
                9 => FieldWriteValueV1::U64(1),
                _ => field.value,
            };
        }
        binding_rows[0].fields = leak(binding_fields);
        binding_tables[0].rows = leak(binding_rows);
        sections[0].tables = leak(binding_tables);

        let schema = portable_object_schema(PortableObjectKind::SemanticDiff);
        let mut entity_row = default_row(schema.sections[1].tables[0].row, false);
        let mut entity_fields = entity_row.fields.to_vec();
        for field in &mut entity_fields {
            field.value = match field.tag {
                2 => FieldWriteValueV1::U16(1),
                10 => FieldWriteValueV1::Bytes(payload),
                _ => field.value,
            };
        }
        entity_row.fields = leak(entity_fields);
        let mut entity_tables = sections[1].tables.to_vec();
        entity_tables[0].rows = leak(vec![entity_row]);
        sections[1].tables = leak(entity_tables);

        ObjectWriteInputV1 {
            kind: input.kind,
            sections: leak(sections),
        }
    }

    fn replace_first_value(
        input: ObjectWriteInputV1<'static>,
        value: FieldWriteValueV1<'static>,
    ) -> ObjectWriteInputV1<'static> {
        let mut sections = input.sections.to_vec();
        let mut tables = sections[0].tables.to_vec();
        let mut rows = tables[0].rows.to_vec();
        let mut fields = rows[0].fields.to_vec();
        fields[0].value = value;
        rows[0].fields = leak(fields);
        tables[0].rows = leak(rows);
        sections[0].tables = leak(tables);
        ObjectWriteInputV1 {
            kind: input.kind,
            sections: leak(sections),
        }
    }

    fn replace_top_level_value(
        input: ObjectWriteInputV1<'static>,
        section_ordinal: usize,
        table_ordinal: usize,
        row_ordinal: usize,
        field_ordinal: usize,
        value: FieldWriteValueV1<'static>,
    ) -> ObjectWriteInputV1<'static> {
        let mut sections = input.sections.to_vec();
        let mut tables = sections[section_ordinal].tables.to_vec();
        let mut rows = tables[table_ordinal].rows.to_vec();
        let mut fields = rows[row_ordinal].fields.to_vec();
        fields[field_ordinal].value = value;
        rows[row_ordinal].fields = leak(fields);
        tables[table_ordinal].rows = leak(rows);
        sections[section_ordinal].tables = leak(tables);
        ObjectWriteInputV1 {
            kind: input.kind,
            sections: leak(sections),
        }
    }

    fn assert_limit_failure_is_atomic(
        input: ObjectWriteInputV1<'static>,
        config: FormatLimitConfig,
        dimension: LimitDimension,
    ) {
        let length = measure_object_v1(input, FormatLimits::V1_HARD).unwrap();
        let mut output = vec![0x6d; usize::try_from(length).unwrap()];
        let before = output.clone();
        let error = encode_object_v1(input, FormatLimits::try_new(config).unwrap(), &mut output)
            .unwrap_err();
        assert_eq!(error.class(), FormatErrorClass::LimitExceeded);
        assert!(matches!(
            error,
            FormatError::LimitExceeded {
                dimension: actual,
                ..
            } if actual == dimension
        ));
        assert_eq!(output, before);
    }

    fn assert_value_bytes(
        value: FieldWriteValueV1<'static>,
        schema: PortableFieldSchema,
        expected: &[u8],
    ) {
        let length = measure_value(
            value,
            schema,
            0,
            FormatLimits::V1_HARD,
            &mut WriteBudget::default(),
        )
        .unwrap();
        assert_eq!(length, expected.len() as u64);
        assert_eq!(raw_value_length(value), length);
        let mut output = vec![0; expected.len()];
        let mut cursor = WriteCursor::new(&mut output);
        write_value(&mut cursor, value);
        assert_eq!(cursor.position(), length);
        assert_eq!(output, expected);
    }

    #[test]
    fn every_object_kind_encodes_an_exact_registry_checked_object() {
        for kind in PortableObjectKind::ALL {
            let input = fixture_object(kind, false);
            let length = measure_object_v1(input, FormatLimits::V1_HARD).unwrap();
            let mut output = vec![0xa5; usize::try_from(length).unwrap()];
            encode_object_v1(input, FormatLimits::V1_HARD, &mut output).unwrap();

            assert_eq!(&output[..4], &kind.magic());
            assert_eq!(
                u64::from_le_bytes(output[24..32].try_into().unwrap()),
                length
            );
            assert_eq!(output.len() as u64, length);
            let view = preflight_object_registry_v1(&output, kind, FormatLimits::V1_HARD).unwrap();
            assert_eq!(view.kind(), kind);
        }
    }

    #[test]
    fn lfcp_minimal_structural_input_has_frozen_exact_header_and_first_section_bytes() {
        let input = fixture_object(PortableObjectKind::CanonicalPublicationDescriptor, false);
        let length = measure_object_v1(input, FormatLimits::V1_HARD).unwrap();
        assert_eq!(length, 719);
        let mut output = vec![0; usize::try_from(length).unwrap()];
        encode_object_v1(input, FormatLimits::V1_HARD, &mut output).unwrap();

        let mut expected_header = Vec::new();
        expected_header.extend_from_slice(b"LFCP");
        expected_header.extend_from_slice(&1_u16.to_le_bytes());
        expected_header.extend_from_slice(&32_u16.to_le_bytes());
        expected_header.extend_from_slice(&0_u32.to_le_bytes());
        expected_header.extend_from_slice(&4_u32.to_le_bytes());
        expected_header.extend_from_slice(&32_u64.to_le_bytes());
        expected_header.extend_from_slice(&719_u64.to_le_bytes());
        for (kind, offset, section_length) in [
            (1_u16, 128_u64, 172_u64),
            (2, 300, 184),
            (3, 484, 138),
            (4, 622, 97),
        ] {
            expected_header.extend_from_slice(&kind.to_le_bytes());
            expected_header.extend_from_slice(&1_u16.to_le_bytes());
            expected_header.extend_from_slice(&0_u32.to_le_bytes());
            expected_header.extend_from_slice(&offset.to_le_bytes());
            expected_header.extend_from_slice(&section_length.to_le_bytes());
        }
        assert_eq!(&output[..128], expected_header);

        let mut expected_section = Vec::new();
        expected_section.extend_from_slice(&1_u32.to_le_bytes());
        expected_section.extend_from_slice(&1_u16.to_le_bytes());
        expected_section.extend_from_slice(&1_u16.to_le_bytes());
        expected_section.extend_from_slice(&1_u32.to_le_bytes());
        expected_section.extend_from_slice(&152_u64.to_le_bytes());
        expected_section.extend_from_slice(&152_u64.to_le_bytes());
        expected_section.extend_from_slice(&5_u32.to_le_bytes());
        expected_section.extend_from_slice(&0_u32.to_le_bytes());
        for (tag, field_type, value) in [
            (1_u16, PortableFieldType::U16, &0_u16.to_le_bytes()[..]),
            (2, PortableFieldType::U16, &0_u16.to_le_bytes()[..]),
            (3, PortableFieldType::Sha256, &[0_u8; 32][..]),
            (4, PortableFieldType::Sha256, &[0_u8; 32][..]),
            (5, PortableFieldType::U64, &0_u64.to_le_bytes()[..]),
        ] {
            expected_section.extend_from_slice(&tag.to_le_bytes());
            expected_section.push(field_type as u8);
            expected_section.push(0);
            expected_section.extend_from_slice(&(value.len() as u64).to_le_bytes());
            expected_section.extend_from_slice(value);
        }
        assert_eq!(&output[128..300], expected_section);
    }

    #[test]
    fn value_checked_lfsd_reaches_the_exact_object_byte_limit() {
        let with_empty_payload = lfsd_with_entity_add_payload(&[]);
        let base_length = measure_object_v1(with_empty_payload, FormatLimits::V1_HARD).unwrap();
        let payload_length = FORMAT_HARD_MAX_OBJECT_BYTES - base_length;
        let payload = leak(vec![0; usize::try_from(payload_length).unwrap()]);
        let exact = lfsd_with_entity_add_payload(payload);
        assert_eq!(
            measure_object_v1(exact, FormatLimits::V1_HARD).unwrap(),
            FORMAT_HARD_MAX_OBJECT_BYTES
        );
        let mut bytes = vec![0; usize::try_from(FORMAT_HARD_MAX_OBJECT_BYTES).unwrap()];
        encode_object_v1(exact, FormatLimits::V1_HARD, &mut bytes).unwrap();
        let framing = crate::preflight_object_framing(
            &bytes,
            PortableObjectKind::SemanticDiff,
            FormatLimits::V1_HARD,
        )
        .unwrap();
        assert_eq!(framing.section(1).unwrap().bytes().len(), 16_776_667);
        preflight_object_values_v1(
            &bytes,
            PortableObjectKind::SemanticDiff,
            FormatLimits::V1_HARD,
        )
        .unwrap();
    }

    #[test]
    fn validation_and_output_length_failures_leave_the_destination_unchanged() {
        let valid = fixture_object(PortableObjectKind::CanonicalPublicationDescriptor, false);
        let length = measure_object_v1(valid, FormatLimits::V1_HARD).unwrap();
        let mut output = vec![0xa5; usize::try_from(length).unwrap()];
        let before = output.clone();

        let invalid = replace_first_value(valid, FieldWriteValueV1::U32(0));
        assert_eq!(
            encode_object_v1(invalid, FormatLimits::V1_HARD, &mut output)
                .unwrap_err()
                .class(),
            FormatErrorClass::BindingMismatch
        );
        assert_eq!(output, before);

        let mut short = vec![0x5a; usize::try_from(length - 1).unwrap()];
        let short_before = short.clone();
        assert_eq!(
            encode_object_v1(valid, FormatLimits::V1_HARD, &mut short)
                .unwrap_err()
                .class(),
            FormatErrorClass::LengthMismatch
        );
        assert_eq!(short, short_before);
    }

    #[test]
    fn nested_records_use_exact_counts_and_writer_limits_before_output() {
        let input = fixture_object(PortableObjectKind::CanonicalArtifact, true);
        let length = measure_object_v1(input, FormatLimits::V1_HARD).unwrap();
        let mut output = vec![0; usize::try_from(length).unwrap()];
        encode_object_v1(input, FormatLimits::V1_HARD, &mut output).unwrap();
        let view = preflight_object_registry_v1(
            &output,
            PortableObjectKind::CanonicalArtifact,
            FormatLimits::V1_HARD,
        )
        .unwrap();
        let mut found_nested_row = false;
        for section_ordinal in 0..view.section_count() {
            let section = view.section(section_ordinal).unwrap();
            for table_ordinal in 0..section.table_count() {
                let table = section.table(table_ordinal).unwrap();
                for row_ordinal in 0..table.row_count() {
                    let row = table.row(row_ordinal).unwrap();
                    for field_ordinal in 0..row.field_count() {
                        if let RegistryCheckedFieldValue::RecordVector(records) =
                            row.field(field_ordinal).unwrap().value().unwrap()
                        {
                            found_nested_row |= records.len() == 1;
                        }
                    }
                }
            }
        }
        assert!(found_nested_row);

        let mut config = FormatLimitConfig::V1_HARD;
        config.max_vector_items = 0;
        let limits = FormatLimits::try_new(config).unwrap();
        let mut untouched = vec![0x3c; usize::try_from(length).unwrap()];
        let before = untouched.clone();
        assert_eq!(
            encode_object_v1(input, limits, &mut untouched)
                .unwrap_err()
                .class(),
            FormatErrorClass::LimitExceeded
        );
        assert_eq!(untouched, before);
    }

    #[test]
    fn writer_enforces_each_applicable_caller_budget_before_output() {
        let lfcp = fixture_object(PortableObjectKind::CanonicalPublicationDescriptor, false);
        let lfcp_length = measure_object_v1(lfcp, FormatLimits::V1_HARD).unwrap();

        let mut config = FormatLimitConfig::V1_HARD;
        config.max_object_bytes = lfcp_length - 1;
        assert_limit_failure_is_atomic(lfcp, config, LimitDimension::ObjectBytes);

        let mut config = FormatLimitConfig::V1_HARD;
        config.max_section_or_table_bytes = 100;
        assert_limit_failure_is_atomic(lfcp, config, LimitDimension::SectionOrTableBytes);

        let mut config = FormatLimitConfig::V1_HARD;
        config.max_rows_per_table = 0;
        assert_limit_failure_is_atomic(lfcp, config, LimitDimension::RowsPerTable);

        let mut config = FormatLimitConfig::V1_HARD;
        config.max_fields_per_row = 0;
        assert_limit_failure_is_atomic(lfcp, config, LimitDimension::FieldsPerRow);

        let lfcp_with_text =
            replace_top_level_value(lfcp, 1, 0, 0, 3, FieldWriteValueV1::Utf8("x"));
        let mut config = FormatLimitConfig::V1_HARD;
        config.max_utf8_field_bytes = 0;
        assert_limit_failure_is_atomic(lfcp_with_text, config, LimitDimension::Utf8FieldBytes);

        let mut config = FormatLimitConfig::V1_HARD;
        config.max_total_utf8_bytes = 0;
        assert_limit_failure_is_atomic(lfcp_with_text, config, LimitDimension::TotalUtf8Bytes);

        let lfca_with_record = fixture_object(PortableObjectKind::CanonicalArtifact, true);
        let mut config = FormatLimitConfig::V1_HARD;
        config.max_vector_items = 0;
        assert_limit_failure_is_atomic(lfca_with_record, config, LimitDimension::VectorItems);

        let mut config = FormatLimitConfig::V1_HARD;
        config.max_total_vector_bytes = 0;
        assert_limit_failure_is_atomic(lfca_with_record, config, LimitDimension::TotalVectorBytes);

        let mut config = FormatLimitConfig::V1_HARD;
        config.max_record_vector_depth = 0;
        assert_limit_failure_is_atomic(lfca_with_record, config, LimitDimension::RecordVectorDepth);
    }

    #[test]
    fn noncanonical_float_bits_and_cumulative_budgets_fail_during_measurement() {
        let float_schema = PortableFieldSchema {
            tag: 1,
            name: "float",
            field_type: PortableFieldType::F32,
            presence: PortableFieldPresence::Required,
            nested_row: None,
        };
        for value in [-0.0_f32, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(
                measure_value(
                    FieldWriteValueV1::F32(value),
                    float_schema,
                    0,
                    FormatLimits::V1_HARD,
                    &mut WriteBudget::default(),
                )
                .unwrap_err()
                .class(),
                FormatErrorClass::NonCanonicalValue
            );
        }

        let utf8_schema = PortableFieldSchema {
            tag: 1,
            name: "text",
            field_type: PortableFieldType::Utf8,
            presence: PortableFieldPresence::Required,
            nested_row: None,
        };
        let mut config = FormatLimitConfig::V1_HARD;
        config.max_total_utf8_bytes = 3;
        let limits = FormatLimits::try_new(config).unwrap();
        let mut budget = WriteBudget::default();
        assert_eq!(
            measure_value(
                FieldWriteValueV1::Utf8("ab"),
                utf8_schema,
                0,
                limits,
                &mut budget,
            ),
            Ok(2)
        );
        assert_eq!(
            measure_value(
                FieldWriteValueV1::Utf8("cd"),
                utf8_schema,
                0,
                limits,
                &mut budget,
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::LimitExceeded
        );
    }

    #[test]
    fn every_field_value_variant_uses_the_measured_exact_little_endian_bytes() {
        const NESTED_FIELDS: &[PortableFieldSchema] = &[PortableFieldSchema {
            tag: 1,
            name: "value",
            field_type: PortableFieldType::U32,
            presence: PortableFieldPresence::Required,
            nested_row: None,
        }];
        const NESTED_ROW: PortableRowSchema = PortableRowSchema {
            fields: NESTED_FIELDS,
            shape: PortableRowShape::Uniform,
        };

        let schema = |field_type, nested_row| PortableFieldSchema {
            tag: 1,
            name: "value",
            field_type,
            presence: PortableFieldPresence::Required,
            nested_row,
        };
        assert_value_bytes(
            FieldWriteValueV1::U8(0x12),
            schema(PortableFieldType::U8, None),
            &[0x12],
        );
        assert_value_bytes(
            FieldWriteValueV1::U16(0x1234),
            schema(PortableFieldType::U16, None),
            &[0x34, 0x12],
        );
        assert_value_bytes(
            FieldWriteValueV1::U32(0x1234_5678),
            schema(PortableFieldType::U32, None),
            &[0x78, 0x56, 0x34, 0x12],
        );
        assert_value_bytes(
            FieldWriteValueV1::U64(0x0123_4567_89ab_cdef),
            schema(PortableFieldType::U64, None),
            &0x0123_4567_89ab_cdef_u64.to_le_bytes(),
        );
        assert_value_bytes(
            FieldWriteValueV1::F32(1.5),
            schema(PortableFieldType::F32, None),
            &1.5_f32.to_bits().to_le_bytes(),
        );
        assert_value_bytes(
            FieldWriteValueV1::F64(-1.5),
            schema(PortableFieldType::F64, None),
            &(-1.5_f64).to_bits().to_le_bytes(),
        );
        assert_value_bytes(
            FieldWriteValueV1::StableId128([0x5a; 16]),
            schema(PortableFieldType::StableId128, None),
            &[0x5a; 16],
        );
        assert_value_bytes(
            FieldWriteValueV1::Sha256([0xa5; 32]),
            schema(PortableFieldType::Sha256, None),
            &[0xa5; 32],
        );
        assert_value_bytes(
            FieldWriteValueV1::Utf8("路"),
            schema(PortableFieldType::Utf8, None),
            "路".as_bytes(),
        );
        assert_value_bytes(
            FieldWriteValueV1::Bytes(&[0xde, 0xad]),
            schema(PortableFieldType::Bytes, None),
            &[0xde, 0xad],
        );
        assert_value_bytes(
            FieldWriteValueV1::OrdinalVectorU32(&[1, 0x1234_5678]),
            schema(PortableFieldType::OrdinalVectorU32, None),
            &[2, 0, 0, 0, 1, 0, 0, 0, 0x78, 0x56, 0x34, 0x12],
        );

        let nested_fields = leak(vec![FieldWriteInputV1 {
            tag: 1,
            value: FieldWriteValueV1::U32(0x1234_5678),
        }]);
        let nested_rows = leak(vec![RowWriteInputV1 {
            fields: nested_fields,
        }]);
        let mut nested_expected = Vec::new();
        nested_expected.extend_from_slice(&1_u32.to_le_bytes());
        nested_expected.extend_from_slice(&32_u64.to_le_bytes());
        nested_expected.extend_from_slice(&1_u32.to_le_bytes());
        nested_expected.extend_from_slice(&0_u32.to_le_bytes());
        nested_expected.extend_from_slice(&1_u16.to_le_bytes());
        nested_expected.push(PortableFieldType::U32 as u8);
        nested_expected.push(0);
        nested_expected.extend_from_slice(&4_u64.to_le_bytes());
        nested_expected.extend_from_slice(&0x1234_5678_u32.to_le_bytes());
        assert_value_bytes(
            FieldWriteValueV1::RecordVector(nested_rows),
            schema(PortableFieldType::RecordVector, Some(&NESTED_ROW)),
            &nested_expected,
        );
        assert_value_bytes(
            FieldWriteValueV1::I32(-2),
            schema(PortableFieldType::I32, None),
            &(-2_i32).to_le_bytes(),
        );
    }
}
