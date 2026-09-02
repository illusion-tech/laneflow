//! 无分配的受限精确编码。
//!
//! 编码器只拥有线格式、附录 A registry 与通用规范值检查。编译器语义投影、跨表闭包、
//! 摘要和文件系统发布事务不属于本模块。

use laneflow_static_contract::{
    CHUNKED_SECTION_PREAMBLE_BYTE_LENGTH, OBJECT_PREAMBLE_BYTE_LENGTH, PortableFieldPresence,
    PortableFieldSchema, PortableFieldType, PortableObjectKind, PortableObjectSchema,
    PortableRowCardinality, PortableRowSchema, PortableRowShape, PortableSectionSchema,
    PortableTableSchema, SECTION_DIRECTORY_ENTRY_BYTE_LENGTH,
    TABLE_CHUNK_DIRECTORY_ENTRY_BYTE_LENGTH, portable_field_mask, portable_object_schema,
};
use sha2::{Digest, Sha256};

use crate::{
    FormatError, FormatLimits, FormatStructure, LimitDimension,
    limits::{CanonicalChunkMetrics, CanonicalRowMetrics, canonical_chunk_with_appended_row},
};

const TABLE_HEADER_BYTES: u64 = 16;
const ROW_HEADER_BYTES: u64 = 16;
const FIELD_HEADER_BYTES: u64 = 12;
const TABLE_SCHEMA_VERSION: u16 = 1;
// 当前封闭对象登记中 LFCA 拥有最多的八个 section。缓存每节 chunk 数可避免百万行
// file-backed 写入在编码阶段再次遍历全部表；若登记扩展出更多 section，下方索引会在测试中
// 直接暴露实现未同步，而不会改变 wire 合同。
const MAX_PREPARED_SECTION_COUNT: usize =
    PortableObjectKind::CanonicalArtifact.section_count() as usize;

/// 一份完整对象的借用写入输入。
#[derive(Clone, Copy, Debug)]
pub struct ObjectWriteInput<'a> {
    pub kind: PortableObjectKind,
    pub sections: &'a [SectionWriteInput<'a>],
}

/// 一个 section 的借用写入输入。
#[derive(Clone, Copy, Debug)]
pub struct SectionWriteInput<'a> {
    pub kind: u16,
    pub tables: &'a [TableWriteInput<'a>],
}

/// 一张 table 的借用写入输入。
#[derive(Clone, Copy, Debug)]
pub struct TableWriteInput<'a> {
    pub kind: u16,
    pub rows: &'a [RowWriteInput<'a>],
}

/// 一行 row 的借用写入输入。
#[derive(Clone, Copy, Debug)]
pub struct RowWriteInput<'a> {
    pub fields: &'a [FieldWriteInput<'a>],
}

/// 一个带显式 registry tag 的 field 写入输入。
#[derive(Clone, Copy, Debug)]
pub struct FieldWriteInput<'a> {
    pub tag: u16,
    pub value: FieldWriteValue<'a>,
}

/// 已完成完整 registry、规范值与资源限制预检的借用写入 capability。
///
/// 该值绑定原始不可变输入和唯一 exact byte length，使调用方可以精确分配后编码，而不再
/// 对同一输入执行第二次全对象预检。它不授予编译器语义或发布真实性。
#[derive(Clone, Copy, Debug)]
pub struct PreparedObject<'a> {
    input: ObjectWriteInput<'a>,
    byte_length: u64,
    format_version: u16,
    section_chunk_counts: [u32; MAX_PREPARED_SECTION_COUNT],
}

impl PreparedObject<'_> {
    /// 编码所需的唯一 exact output length。
    #[must_use]
    pub const fn byte_len(self) -> u64 {
        self.byte_length
    }
}

/// 封闭 field type 的有类型写入值。
#[derive(Clone, Copy, Debug)]
pub enum FieldWriteValue<'a> {
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
    RecordVector(&'a [RowWriteInput<'a>]),
    I32(i32),
}

impl FieldWriteValue<'_> {
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

#[derive(Clone, Copy, Debug)]
struct MeasuredTable {
    chunk_count: u32,
    chunks_byte_length: u64,
}

#[derive(Clone, Copy, Debug)]
struct MeasuredSection {
    chunk_count: u32,
    byte_length: u64,
}

#[derive(Clone, Copy, Debug)]
struct MeasuredObject {
    byte_length: u64,
    section_chunk_counts: [u32; MAX_PREPARED_SECTION_COUNT],
}

/// 校验完整输入并返回唯一的 exact object byte length。
///
/// 本函数不分配、不写入，也不执行编译器语义或跨对象绑定验证。
pub fn measure_object(
    input: ObjectWriteInput<'_>,
    limits: FormatLimits,
) -> Result<u64, FormatError> {
    Ok(measure_object_with_schema(input, limits, portable_object_schema(input.kind))?.byte_length)
}

fn measure_object_with_schema(
    input: ObjectWriteInput<'_>,
    limits: FormatLimits,
    schema: &'static PortableObjectSchema,
) -> Result<MeasuredObject, FormatError> {
    check_exact_count(
        FormatStructure::SectionDirectory,
        input.sections.len(),
        schema.sections.len(),
    )?;

    let mut object_length = input.kind.first_section_offset();
    let mut section_chunk_counts = [0_u32; MAX_PREPARED_SECTION_COUNT];
    for (ordinal, (section, section_schema)) in
        input.sections.iter().zip(schema.sections).enumerate()
    {
        check_ordered_kind(
            FormatStructure::SectionDirectory,
            section.kind,
            section_schema.kind,
            ordinal,
        )?;
        let measured = measure_section(input.kind, *section, section_schema, limits)?;
        *section_chunk_counts
            .get_mut(ordinal)
            .expect("portable object registry fits prepared section count cache") =
            measured.chunk_count;
        object_length = checked_add(
            object_length,
            measured.byte_length,
            FormatStructure::ObjectPreamble,
        )?;
        check_limit(
            LimitDimension::ObjectBytes,
            object_length,
            limits.config().max_object_bytes,
        )?;
    }
    Ok(MeasuredObject {
        byte_length: object_length,
        section_chunk_counts,
    })
}

/// 完成一次全对象预检，并返回可重复使用的 exact-length 编码 capability。
pub fn prepare_object<'a>(
    input: ObjectWriteInput<'a>,
    limits: FormatLimits,
) -> Result<PreparedObject<'a>, FormatError> {
    prepare_object_with_schema(
        input,
        limits,
        portable_object_schema(input.kind),
        input.kind.format_version(),
    )
}

fn prepare_object_with_schema<'a>(
    input: ObjectWriteInput<'a>,
    limits: FormatLimits,
    schema: &'static PortableObjectSchema,
    format_version: u16,
) -> Result<PreparedObject<'a>, FormatError> {
    let measured = measure_object_with_schema(input, limits, schema)?;
    Ok(PreparedObject {
        input,
        byte_length: measured.byte_length,
        format_version,
        section_chunk_counts: measured.section_chunk_counts,
    })
}

/// 把已经预检的输入精确编码到调用方提供的缓冲区。
///
/// 缓冲区长度不精确时在写入前失败；成功时不重复执行 registry、规范值或资源限制预检。
pub fn encode_prepared_object(
    prepared: PreparedObject<'_>,
    output: &mut [u8],
) -> Result<(), FormatError> {
    let output_length = output.len() as u64;
    if output_length != prepared.byte_len() {
        return Err(FormatError::LengthMismatch {
            structure: FormatStructure::ObjectPreamble,
            declared: output_length,
            actual: prepared.byte_len(),
        });
    }

    let mut cursor = WriteCursor::new(SliceSink::new(output));
    let result = write_object(
        &mut cursor,
        prepared.input,
        prepared.byte_len(),
        prepared.format_version,
        prepared.section_chunk_counts,
    );
    match result {
        Ok(()) => {}
        Err(error) => match error {},
    }
    debug_assert_eq!(cursor.position(), output_length);
    Ok(())
}

/// 把完整输入精确编码到调用方提供的缓冲区。
///
/// 编码器先完成与 [`measure_object`] 相同的全对象预检，并在缓冲区长度不精确时直接
/// 失败。任何返回的错误都发生在写入开始前，因此 `output` 保持逐字节不变。成功时整个
/// `output` 恰好是一份无 padding、无尾字节的对象。
pub fn encode_object(
    input: ObjectWriteInput<'_>,
    limits: FormatLimits,
    output: &mut [u8],
) -> Result<(), FormatError> {
    encode_prepared_object(prepare_object(input, limits)?, output)
}

fn measure_section(
    object_kind: PortableObjectKind,
    section: SectionWriteInput<'_>,
    schema: &'static PortableSectionSchema,
    limits: FormatLimits,
) -> Result<MeasuredSection, FormatError> {
    check_exact_count(
        FormatStructure::Section,
        section.tables.len(),
        schema.tables.len(),
    )?;
    if object_kind == PortableObjectKind::CanonicalPublicationDescriptor {
        let [table] = section.tables else {
            return Err(FormatError::BindingMismatch {
                structure: FormatStructure::Section,
            });
        };
        let [table_schema] = schema.tables else {
            return Err(FormatError::BindingMismatch {
                structure: FormatStructure::Section,
            });
        };
        check_ordered_kind(FormatStructure::Section, table.kind, table_schema.kind, 0)?;
        let measured = measure_table(object_kind, section.kind, *table, table_schema, limits)?;
        if measured.chunk_count != 1 {
            return Err(FormatError::BindingMismatch {
                structure: FormatStructure::TableRows,
            });
        }
        return Ok(MeasuredSection {
            chunk_count: measured.chunk_count,
            byte_length: measured.chunks_byte_length,
        });
    }

    let mut section_length = CHUNKED_SECTION_PREAMBLE_BYTE_LENGTH;
    let mut chunk_count = 0_u32;
    for (ordinal, (table, table_schema)) in section.tables.iter().zip(schema.tables).enumerate() {
        check_ordered_kind(
            FormatStructure::Section,
            table.kind,
            table_schema.kind,
            ordinal,
        )?;
        let measured = measure_table(object_kind, section.kind, *table, table_schema, limits)?;
        chunk_count = chunk_count.checked_add(measured.chunk_count).ok_or(
            FormatError::ArithmeticOverflow {
                structure: FormatStructure::ChunkDirectory,
            },
        )?;
        section_length = checked_add(
            section_length,
            measured.chunks_byte_length,
            FormatStructure::Section,
        )?;
    }
    check_limit(
        LimitDimension::ChunksPerSection,
        u64::from(chunk_count),
        u64::from(limits.max_chunks_per_section()),
    )?;
    section_length = checked_add(
        section_length,
        u64::from(chunk_count)
            .checked_mul(TABLE_CHUNK_DIRECTORY_ENTRY_BYTE_LENGTH)
            .ok_or(FormatError::ArithmeticOverflow {
                structure: FormatStructure::ChunkDirectory,
            })?,
        FormatStructure::Section,
    )?;
    Ok(MeasuredSection {
        chunk_count,
        byte_length: section_length,
    })
}

fn measure_table(
    object_kind: PortableObjectKind,
    section_kind: u16,
    table: TableWriteInput<'_>,
    schema: &'static PortableTableSchema,
    limits: FormatLimits,
) -> Result<MeasuredTable, FormatError> {
    let row_count = count_u32(table.rows.len(), FormatStructure::TableRows)?;
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

    if row_count == 0 {
        return Ok(MeasuredTable {
            chunk_count: 0,
            chunks_byte_length: 0,
        });
    }

    let mut chunk_count = 1_u32;
    let mut chunks_byte_length = 0_u64;
    let mut chunk = CanonicalChunkMetrics::empty(TABLE_HEADER_BYTES);
    for row in table.rows {
        let mut row_budget = WriteBudget::default();
        let row_length = measure_row(*row, schema.row, 0, limits, &mut row_budget)?;
        let single_row_chunk_length =
            checked_add(TABLE_HEADER_BYTES, row_length, FormatStructure::Table)?;
        check_limit(
            LimitDimension::TableChunkBytes,
            single_row_chunk_length,
            limits.config().max_table_chunk_bytes,
        )?;
        let row_metrics = CanonicalRowMetrics {
            exact_byte_length: row_length,
            total_utf8_bytes: row_budget.total_utf8_bytes,
            total_vector_bytes: row_budget.total_vector_bytes,
        };
        if let Some(next) = canonical_chunk_with_appended_row(
            object_kind,
            section_kind,
            table.kind,
            chunk,
            row_metrics,
        ) {
            chunk = next;
        } else {
            check_caller_chunk_budget(object_kind, section_kind, table.kind, chunk, limits)?;
            chunks_byte_length = checked_add(
                chunks_byte_length,
                chunk.exact_byte_length,
                FormatStructure::Section,
            )?;
            chunk_count = chunk_count
                .checked_add(1)
                .ok_or(FormatError::ArithmeticOverflow {
                    structure: FormatStructure::ChunkDirectory,
                })?;
            chunk = canonical_chunk_with_appended_row(
                object_kind,
                section_kind,
                table.kind,
                CanonicalChunkMetrics::empty(TABLE_HEADER_BYTES),
                row_metrics,
            )
            .ok_or(FormatError::LimitExceeded {
                dimension: LimitDimension::TableChunkBytes,
                actual: single_row_chunk_length,
                limit: limits.config().max_table_chunk_bytes,
            })?;
        }
    }
    check_caller_chunk_budget(object_kind, section_kind, table.kind, chunk, limits)?;
    chunks_byte_length = checked_add(
        chunks_byte_length,
        chunk.exact_byte_length,
        FormatStructure::Section,
    )?;
    Ok(MeasuredTable {
        chunk_count,
        chunks_byte_length,
    })
}

fn check_caller_chunk_budget(
    object_kind: PortableObjectKind,
    section_kind: u16,
    table_kind: u16,
    chunk: CanonicalChunkMetrics,
    limits: FormatLimits,
) -> Result<(), FormatError> {
    let config = limits.config();
    check_limit(
        LimitDimension::TableChunkBytes,
        chunk.exact_byte_length,
        config.max_table_chunk_bytes,
    )?;
    check_limit(
        LimitDimension::RowsPerChunk,
        u64::from(chunk.row_count),
        u64::from(config.max_rows_per_chunk),
    )?;
    if object_kind == PortableObjectKind::SourceMap && section_kind == 2 && table_kind == 3 {
        check_limit(
            LimitDimension::SourceLocationRowsPerChunk,
            u64::from(chunk.row_count),
            u64::from(config.max_source_location_rows_per_chunk),
        )?;
    }
    check_limit(
        LimitDimension::TotalUtf8Bytes,
        chunk.total_utf8_bytes,
        config.max_total_utf8_bytes,
    )?;
    check_limit(
        LimitDimension::TotalVectorBytes,
        chunk.total_vector_bytes,
        config.max_total_vector_bytes,
    )
}

fn measure_row(
    row: RowWriteInput<'_>,
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
        if let Some((tag, _)) = schema.shape.discriminant()
            && field.tag == tag
            && let FieldWriteValue::U8(value) = field.value
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
    if schema.shape == PortableRowShape::PolicyLocalChange {
        let embedded = policy_value_budget(row, limits)?;
        budget.total_utf8_bytes = checked_add(
            budget.total_utf8_bytes,
            embedded.total_utf8_bytes,
            FormatStructure::Row,
        )?;
        check_limit(
            LimitDimension::TotalUtf8Bytes,
            budget.total_utf8_bytes,
            limits.config().max_total_utf8_bytes,
        )?;
        budget.total_vector_bytes = checked_total_vector_bytes(
            budget.total_vector_bytes,
            embedded.total_vector_bytes,
            limits,
        )?;
    }
    Ok(row_length)
}

fn policy_value_budget(
    row: RowWriteInput<'_>,
    limits: FormatLimits,
) -> Result<crate::table::PreflightBudget, FormatError> {
    let kind = row
        .fields
        .iter()
        .find_map(|f| match (f.tag, f.value) {
            (3, FieldWriteValue::U8(v)) => Some(v),
            _ => None,
        })
        .ok_or(FormatError::BindingMismatch {
            structure: FormatStructure::RowFields,
        })?;
    let mut budget = crate::table::PreflightBudget::default();
    for field in row.fields {
        if matches!(field.tag, 5 | 6)
            && let FieldWriteValue::Bytes(bytes) = field.value
        {
            crate::policy_value::preflight_member_value(kind, bytes, limits, &mut budget)?;
        }
    }
    Ok(budget)
}

fn measure_value(
    value: FieldWriteValue<'_>,
    schema: PortableFieldSchema,
    depth: u8,
    limits: FormatLimits,
    budget: &mut WriteBudget,
) -> Result<u64, FormatError> {
    let value_length = match value {
        FieldWriteValue::U8(_) => 1,
        FieldWriteValue::U16(_) => 2,
        FieldWriteValue::U32(_) | FieldWriteValue::F32(_) | FieldWriteValue::I32(_) => 4,
        FieldWriteValue::U64(_) | FieldWriteValue::F64(_) => 8,
        FieldWriteValue::StableId128(_) => 16,
        FieldWriteValue::Sha256(_) => 32,
        FieldWriteValue::Utf8(value) => {
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
        FieldWriteValue::Bytes(value) => value.len() as u64,
        FieldWriteValue::OrdinalVectorU32(items) => {
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
        FieldWriteValue::RecordVector(rows) => {
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
            let mut pending = *budget;
            let mut length = 4_u64;
            checked_total_vector_bytes(pending.total_vector_bytes, length, limits)?;
            for row in rows {
                length = checked_add(
                    length,
                    measure_row(*row, nested_schema, next_depth, limits, &mut pending)?,
                    FormatStructure::RecordVector,
                )?;
                checked_total_vector_bytes(pending.total_vector_bytes, length, limits)?;
            }
            pending.total_vector_bytes =
                checked_total_vector_bytes(pending.total_vector_bytes, length, limits)?;
            *budget = pending;
            return Ok(length);
        }
    };

    match value {
        FieldWriteValue::F32(value) if !canonical_f32(value) => {
            return Err(noncanonical_value());
        }
        FieldWriteValue::F64(value) if !canonical_f64(value) => {
            return Err(noncanonical_value());
        }
        _ => {}
    }

    if matches!(value, FieldWriteValue::OrdinalVectorU32(_)) {
        budget.total_vector_bytes =
            checked_total_vector_bytes(budget.total_vector_bytes, value_length, limits)?;
    }
    Ok(value_length)
}

fn checked_total_vector_bytes(
    current: u64,
    additional: u64,
    limits: FormatLimits,
) -> Result<u64, FormatError> {
    let total = checked_add(current, additional, FormatStructure::FieldValue)?;
    check_limit(
        LimitDimension::TotalVectorBytes,
        total,
        limits.config().max_total_vector_bytes,
    )?;
    Ok(total)
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
        shape => {
            let (_, variants) = shape.discriminant().expect("non-uniform row shape");
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
) -> Result<(), FormatError> {
    // Table kind codes are registry facts and may gain reserved holes in a later
    // registry revision. Match the registered code, not 1..=table_count.
    if actual == 0 {
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

fn encoded_row_measure(row: RowWriteInput<'_>) -> (u64, WriteBudget) {
    let mut length = ROW_HEADER_BYTES;
    let mut budget = WriteBudget::default();
    for field in row.fields {
        let (value_length, value_budget) = encoded_value_measure(field.value);
        length += FIELD_HEADER_BYTES + value_length;
        budget.total_utf8_bytes += value_budget.total_utf8_bytes;
        budget.total_vector_bytes += value_budget.total_vector_bytes;
    }
    (length, budget)
}

fn encoded_value_measure(value: FieldWriteValue<'_>) -> (u64, WriteBudget) {
    let mut budget = WriteBudget::default();
    let length = match value {
        FieldWriteValue::U8(_) => 1,
        FieldWriteValue::U16(_) => 2,
        FieldWriteValue::U32(_) | FieldWriteValue::F32(_) | FieldWriteValue::I32(_) => 4,
        FieldWriteValue::U64(_) | FieldWriteValue::F64(_) => 8,
        FieldWriteValue::StableId128(_) => 16,
        FieldWriteValue::Sha256(_) => 32,
        FieldWriteValue::Utf8(value) => {
            budget.total_utf8_bytes = value.len() as u64;
            value.len() as u64
        }
        FieldWriteValue::Bytes(value) => value.len() as u64,
        FieldWriteValue::OrdinalVectorU32(items) => {
            let length = 4 + items.len() as u64 * 4;
            budget.total_vector_bytes = length;
            length
        }
        FieldWriteValue::RecordVector(rows) => {
            let mut length = 4_u64;
            for row in rows {
                let (row_length, row_budget) = encoded_row_measure(*row);
                length += row_length;
                budget.total_utf8_bytes += row_budget.total_utf8_bytes;
                budget.total_vector_bytes += row_budget.total_vector_bytes;
            }
            budget.total_vector_bytes += length;
            length
        }
    };
    (length, budget)
}

fn next_chunk_end(
    object_kind: PortableObjectKind,
    section_kind: u16,
    table: TableWriteInput<'_>,
    first_row: usize,
) -> (usize, CanonicalChunkMetrics) {
    let mut end = first_row;
    let mut chunk = CanonicalChunkMetrics::empty(TABLE_HEADER_BYTES);
    while end < table.rows.len() {
        let (row_length, mut row_budget) = encoded_row_measure(table.rows[end]);
        if object_kind == PortableObjectKind::SemanticDiff && section_kind == 7 {
            let embedded = policy_value_budget(table.rows[end], FormatLimits::HARD)
                .expect("prepared immutable policy value");
            row_budget.total_utf8_bytes += embedded.total_utf8_bytes;
            row_budget.total_vector_bytes += embedded.total_vector_bytes;
        }
        let row = CanonicalRowMetrics {
            exact_byte_length: row_length,
            total_utf8_bytes: row_budget.total_utf8_bytes,
            total_vector_bytes: row_budget.total_vector_bytes,
        };
        let Some(next) =
            canonical_chunk_with_appended_row(object_kind, section_kind, table.kind, chunk, row)
        else {
            debug_assert!(
                chunk.row_count != 0,
                "prepared row must fit an empty hard chunk"
            );
            break;
        };
        chunk = next;
        end += 1;
    }
    (end, chunk)
}

fn write_object<S: ObjectWriteSink>(
    cursor: &mut WriteCursor<S>,
    input: ObjectWriteInput<'_>,
    object_length: u64,
    format_version: u16,
    section_chunk_counts: [u32; MAX_PREPARED_SECTION_COUNT],
) -> Result<(), S::Error> {
    cursor.put(&input.kind.magic())?;
    cursor.u16(format_version)?;
    cursor.u16(OBJECT_PREAMBLE_BYTE_LENGTH)?;
    cursor.u32(0)?;
    cursor.u32(input.kind.section_count())?;
    cursor.u64(u64::from(OBJECT_PREAMBLE_BYTE_LENGTH))?;
    cursor.u64(object_length)?;

    for section in input.sections {
        cursor.u16(section.kind)?;
        cursor.u16(input.kind.section_format_version())?;
        cursor.u32(0)?;
        cursor.u64(0)?;
        cursor.u64(0)?;
    }

    for (ordinal, section) in input.sections.iter().enumerate() {
        let section_offset = cursor.position();
        write_section(cursor, input.kind, *section, section_chunk_counts[ordinal])?;
        let section_length = cursor.position() - section_offset;
        let directory_entry = u64::from(OBJECT_PREAMBLE_BYTE_LENGTH)
            + u64::try_from(ordinal).expect("prepared section ordinal fits u64")
                * SECTION_DIRECTORY_ENTRY_BYTE_LENGTH;
        cursor.patch_u64(directory_entry + 8, section_offset)?;
        cursor.patch_u64(directory_entry + 16, section_length)?;
    }
    Ok(())
}

fn write_section<S: ObjectWriteSink>(
    cursor: &mut WriteCursor<S>,
    object_kind: PortableObjectKind,
    section: SectionWriteInput<'_>,
    chunk_count: u32,
) -> Result<(), S::Error> {
    if object_kind == PortableObjectKind::CanonicalPublicationDescriptor {
        debug_assert_eq!(chunk_count, 1);
        return write_table(cursor, section.tables[0]);
    }

    cursor.u32(chunk_count)?;
    cursor.u16(TABLE_CHUNK_DIRECTORY_ENTRY_BYTE_LENGTH as u16)?;
    cursor.u16(0)?;
    let directory_byte_length = CHUNKED_SECTION_PREAMBLE_BYTE_LENGTH
        + u64::from(chunk_count) * TABLE_CHUNK_DIRECTORY_ENTRY_BYTE_LENGTH;
    cursor.u64(directory_byte_length)?;
    let directory_start = cursor.position();
    cursor.reserve_bytes(
        usize::try_from(u64::from(chunk_count) * TABLE_CHUNK_DIRECTORY_ENTRY_BYTE_LENGTH)
            .expect("prepared object directory fits the output address space"),
    )?;

    let section_start = directory_start - CHUNKED_SECTION_PREAMBLE_BYTE_LENGTH;
    let mut directory_ordinal = 0_u32;
    for table in section.tables {
        let mut first_row = 0_usize;
        let mut chunk_index = 0_u32;
        while first_row < table.rows.len() {
            let (end_row, chunk) = next_chunk_end(object_kind, section.kind, *table, first_row);
            let chunk_offset = cursor.position();
            cursor.begin_digest();
            write_table_rows(
                cursor,
                table.kind,
                &table.rows[first_row..end_row],
                chunk.exact_byte_length - TABLE_HEADER_BYTES,
            )?;
            let chunk_byte_length = cursor.position() - chunk_offset;
            debug_assert_eq!(chunk_byte_length, chunk.exact_byte_length);
            let digest = cursor.end_digest();
            let entry = directory_start
                + u64::from(directory_ordinal) * TABLE_CHUNK_DIRECTORY_ENTRY_BYTE_LENGTH;
            cursor.patch_u16(entry, table.kind)?;
            cursor.patch_u16(entry + 2, TABLE_SCHEMA_VERSION)?;
            cursor.patch_u32(entry + 4, chunk_index)?;
            cursor.patch_u32(
                entry + 8,
                u32::try_from(first_row).expect("prepared logical row ordinal fits u32"),
            )?;
            cursor.patch_u32(
                entry + 12,
                u32::try_from(end_row - first_row).expect("prepared chunk row count fits u32"),
            )?;
            cursor.patch_u32(entry + 16, 0)?;
            cursor.patch_u32(entry + 20, 0)?;
            cursor.patch_u64(entry + 24, chunk_offset - section_start)?;
            cursor.patch_u64(entry + 32, chunk_byte_length)?;
            cursor.patch_bytes(entry + 40, &digest)?;
            directory_ordinal += 1;
            chunk_index += 1;
            first_row = end_row;
        }
    }
    debug_assert_eq!(directory_ordinal, chunk_count);
    Ok(())
}

fn write_table<S: ObjectWriteSink>(
    cursor: &mut WriteCursor<S>,
    table: TableWriteInput<'_>,
) -> Result<(), S::Error> {
    let rows_byte_length = table
        .rows
        .iter()
        .map(|row| encoded_row_measure(*row).0)
        .sum();
    write_table_rows(cursor, table.kind, table.rows, rows_byte_length)
}

fn write_table_rows<S: ObjectWriteSink>(
    cursor: &mut WriteCursor<S>,
    kind: u16,
    rows: &[RowWriteInput<'_>],
    rows_byte_length: u64,
) -> Result<(), S::Error> {
    cursor.u16(kind)?;
    cursor.u16(TABLE_SCHEMA_VERSION)?;
    cursor.u32(rows.len() as u32)?;
    cursor.u64(rows_byte_length)?;
    for row in rows {
        write_row(cursor, *row)?;
    }
    Ok(())
}

fn write_row<S: ObjectWriteSink>(
    cursor: &mut WriteCursor<S>,
    row: RowWriteInput<'_>,
) -> Result<(), S::Error> {
    cursor.u64(encoded_row_measure(row).0)?;
    cursor.u32(row.fields.len() as u32)?;
    cursor.u32(0)?;
    for field in row.fields {
        cursor.u16(field.tag)?;
        cursor.u8(field.value.field_type() as u8)?;
        cursor.u8(0)?;
        cursor.u64(encoded_value_measure(field.value).0)?;
        write_value(cursor, field.value)?;
    }
    Ok(())
}

fn write_value<S: ObjectWriteSink>(
    cursor: &mut WriteCursor<S>,
    value: FieldWriteValue<'_>,
) -> Result<(), S::Error> {
    match value {
        FieldWriteValue::U8(value) => cursor.u8(value)?,
        FieldWriteValue::U16(value) => cursor.u16(value)?,
        FieldWriteValue::U32(value) => cursor.u32(value)?,
        FieldWriteValue::U64(value) => cursor.u64(value)?,
        FieldWriteValue::F32(value) => cursor.u32(value.to_bits())?,
        FieldWriteValue::F64(value) => cursor.u64(value.to_bits())?,
        FieldWriteValue::StableId128(value) => cursor.put(&value)?,
        FieldWriteValue::Sha256(value) => cursor.put(&value)?,
        FieldWriteValue::Utf8(value) => cursor.put(value.as_bytes())?,
        FieldWriteValue::Bytes(value) => cursor.put(value)?,
        FieldWriteValue::OrdinalVectorU32(items) => {
            cursor.u32(items.len() as u32)?;
            for item in items {
                cursor.u32(*item)?;
            }
        }
        FieldWriteValue::RecordVector(rows) => {
            cursor.u32(rows.len() as u32)?;
            for row in rows {
                write_row(cursor, *row)?;
            }
        }
        FieldWriteValue::I32(value) => cursor.put(&value.to_le_bytes())?,
    }
    Ok(())
}

pub(crate) trait ObjectWriteSink {
    type Error;

    fn position(&self) -> u64;
    fn write_exact(&mut self, bytes: &[u8]) -> Result<(), Self::Error>;
    fn patch_exact_at(&mut self, offset: u64, bytes: &[u8]) -> Result<(), Self::Error>;

    #[cfg(feature = "std")]
    fn finish(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

struct SliceSink<'a> {
    output: &'a mut [u8],
    position: usize,
}

impl<'a> SliceSink<'a> {
    const fn new(output: &'a mut [u8]) -> Self {
        Self {
            output,
            position: 0,
        }
    }
}

impl ObjectWriteSink for SliceSink<'_> {
    type Error = core::convert::Infallible;

    fn position(&self) -> u64 {
        self.position as u64
    }

    fn write_exact(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        let end = self.position + bytes.len();
        self.output[self.position..end].copy_from_slice(bytes);
        self.position = end;
        Ok(())
    }

    fn patch_exact_at(&mut self, offset: u64, bytes: &[u8]) -> Result<(), Self::Error> {
        let offset = usize::try_from(offset).expect("prepared output offset fits usize");
        self.output[offset..offset + bytes.len()].copy_from_slice(bytes);
        Ok(())
    }
}

pub(crate) struct WriteCursor<S> {
    sink: S,
    digest: Option<Sha256>,
}

impl<S: ObjectWriteSink> WriteCursor<S> {
    pub(crate) const fn new(sink: S) -> Self {
        Self { sink, digest: None }
    }

    pub(crate) fn position(&self) -> u64 {
        self.sink.position()
    }

    fn put(&mut self, bytes: &[u8]) -> Result<(), S::Error> {
        if let Some(digest) = &mut self.digest {
            digest.update(bytes);
        }
        self.sink.write_exact(bytes)
    }

    fn u8(&mut self, value: u8) -> Result<(), S::Error> {
        self.put(&[value])
    }

    fn u16(&mut self, value: u16) -> Result<(), S::Error> {
        self.put(&value.to_le_bytes())
    }

    fn u32(&mut self, value: u32) -> Result<(), S::Error> {
        self.put(&value.to_le_bytes())
    }

    fn u64(&mut self, value: u64) -> Result<(), S::Error> {
        self.put(&value.to_le_bytes())
    }

    fn reserve_bytes(&mut self, mut byte_length: usize) -> Result<(), S::Error> {
        const ZEROES: [u8; 256] = [0; 256];
        while byte_length != 0 {
            let step = byte_length.min(ZEROES.len());
            self.put(&ZEROES[..step])?;
            byte_length -= step;
        }
        Ok(())
    }

    fn patch_u16(&mut self, offset: u64, value: u16) -> Result<(), S::Error> {
        self.sink.patch_exact_at(offset, &value.to_le_bytes())
    }

    fn patch_u32(&mut self, offset: u64, value: u32) -> Result<(), S::Error> {
        self.sink.patch_exact_at(offset, &value.to_le_bytes())
    }

    fn patch_bytes(&mut self, offset: u64, value: &[u8]) -> Result<(), S::Error> {
        self.sink.patch_exact_at(offset, value)
    }

    fn patch_u64(&mut self, offset: u64, value: u64) -> Result<(), S::Error> {
        self.sink.patch_exact_at(offset, &value.to_le_bytes())
    }

    fn begin_digest(&mut self) {
        debug_assert!(self.digest.is_none());
        self.digest = Some(Sha256::new());
    }

    fn end_digest(&mut self) -> [u8; 32] {
        self.digest
            .take()
            .expect("prepared chunk digest has a matching begin")
            .finalize()
            .into()
    }

    #[cfg(feature = "std")]
    fn finish(&mut self) -> Result<(), S::Error> {
        self.sink.finish()
    }
}

#[cfg(feature = "std")]
pub(crate) fn encode_prepared_object_to_sink<S: ObjectWriteSink>(
    prepared: PreparedObject<'_>,
    sink: S,
) -> Result<(), S::Error> {
    let mut cursor = WriteCursor::new(sink);
    write_object(
        &mut cursor,
        prepared.input,
        prepared.byte_len(),
        prepared.format_version,
        prepared.section_chunk_counts,
    )?;
    debug_assert_eq!(cursor.position(), prepared.byte_len());
    cursor.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{boxed::Box, vec, vec::Vec};

    use laneflow_static_contract::{
        FORMAT_HARD_MAX_ROWS_PER_CHUNK, PortableFieldPresence, PortableRowCardinality,
        PortableRowShape, portable_object_schema,
    };

    use super::*;
    use crate::{
        FormatErrorClass, FormatLimitConfig, RegistryCheckedFieldValue, preflight_object_registry,
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
            shape => {
                let (tag, variants) = shape.discriminant().unwrap();
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
    ) -> FieldWriteValue<'static> {
        match field.field_type {
            PortableFieldType::U8 => FieldWriteValue::U8(
                discriminant
                    .filter(|(tag, _)| *tag == field.tag)
                    .map_or(0, |(_, value)| value),
            ),
            PortableFieldType::U16 => FieldWriteValue::U16(0),
            PortableFieldType::U32 => FieldWriteValue::U32(0),
            PortableFieldType::U64 => FieldWriteValue::U64(0),
            PortableFieldType::F32 => FieldWriteValue::F32(0.0),
            PortableFieldType::F64 => FieldWriteValue::F64(0.0),
            PortableFieldType::StableId128 => FieldWriteValue::StableId128([0; 16]),
            PortableFieldType::Sha256 => FieldWriteValue::Sha256([0; 32]),
            PortableFieldType::Utf8 => FieldWriteValue::Utf8(""),
            PortableFieldType::Bytes => FieldWriteValue::Bytes(&[]),
            PortableFieldType::OrdinalVectorU32 => FieldWriteValue::OrdinalVectorU32(&[]),
            PortableFieldType::RecordVector => {
                if populate_records {
                    let nested = leak(vec![default_row(
                        field.nested_row.expect("registry supplies nested row"),
                        false,
                    )]);
                    FieldWriteValue::RecordVector(nested)
                } else {
                    FieldWriteValue::RecordVector(&[])
                }
            }
            PortableFieldType::I32 => FieldWriteValue::I32(0),
        }
    }

    fn default_row(
        schema: &'static PortableRowSchema,
        populate_records: bool,
    ) -> RowWriteInput<'static> {
        let (selected, discriminant) = selected_row_fields(schema);
        let fields = schema
            .fields
            .iter()
            .filter(|field| selected & portable_field_mask(field.tag) != 0)
            .map(|field| FieldWriteInput {
                tag: field.tag,
                value: default_value(field, discriminant, populate_records),
            })
            .collect();
        RowWriteInput {
            fields: leak(fields),
        }
    }

    fn fixture_object(
        kind: PortableObjectKind,
        populate_first_record_vector: bool,
    ) -> ObjectWriteInput<'static> {
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
                        TableWriteInput {
                            kind: table.kind,
                            rows,
                        }
                    })
                    .collect();
                SectionWriteInput {
                    kind: section.kind,
                    tables: leak(tables),
                }
            })
            .collect();
        assert!(!populate_first_record_vector || record_populated);
        ObjectWriteInput {
            kind,
            sections: leak(sections),
        }
    }

    fn replace_table_rows(
        input: ObjectWriteInput<'static>,
        section_ordinal: usize,
        table_ordinal: usize,
        rows: &'static [RowWriteInput<'static>],
    ) -> ObjectWriteInput<'static> {
        let mut sections = input.sections.to_vec();
        let mut tables = sections[section_ordinal].tables.to_vec();
        tables[table_ordinal].rows = rows;
        sections[section_ordinal].tables = leak(tables);
        ObjectWriteInput {
            kind: input.kind,
            sections: leak(sections),
        }
    }

    fn replace_first_value(
        input: ObjectWriteInput<'static>,
        value: FieldWriteValue<'static>,
    ) -> ObjectWriteInput<'static> {
        let mut sections = input.sections.to_vec();
        let mut tables = sections[0].tables.to_vec();
        let mut rows = tables[0].rows.to_vec();
        let mut fields = rows[0].fields.to_vec();
        fields[0].value = value;
        rows[0].fields = leak(fields);
        tables[0].rows = leak(rows);
        sections[0].tables = leak(tables);
        ObjectWriteInput {
            kind: input.kind,
            sections: leak(sections),
        }
    }

    fn replace_top_level_value(
        input: ObjectWriteInput<'static>,
        section_ordinal: usize,
        table_ordinal: usize,
        row_ordinal: usize,
        field_ordinal: usize,
        value: FieldWriteValue<'static>,
    ) -> ObjectWriteInput<'static> {
        let mut sections = input.sections.to_vec();
        let mut tables = sections[section_ordinal].tables.to_vec();
        let mut rows = tables[table_ordinal].rows.to_vec();
        let mut fields = rows[row_ordinal].fields.to_vec();
        fields[field_ordinal].value = value;
        rows[row_ordinal].fields = leak(fields);
        tables[table_ordinal].rows = leak(rows);
        sections[section_ordinal].tables = leak(tables);
        ObjectWriteInput {
            kind: input.kind,
            sections: leak(sections),
        }
    }

    fn assert_limit_failure_is_atomic(
        input: ObjectWriteInput<'static>,
        config: FormatLimitConfig,
        dimension: LimitDimension,
    ) {
        let length = measure_object(input, FormatLimits::HARD).unwrap();
        let mut output = vec![0x6d; usize::try_from(length).unwrap()];
        let before = output.clone();
        let error =
            encode_object(input, FormatLimits::try_new(config).unwrap(), &mut output).unwrap_err();
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
        value: FieldWriteValue<'static>,
        schema: PortableFieldSchema,
        expected: &[u8],
    ) {
        let length = measure_value(
            value,
            schema,
            0,
            FormatLimits::HARD,
            &mut WriteBudget::default(),
        )
        .unwrap();
        assert_eq!(length, expected.len() as u64);
        let mut output = vec![0; expected.len()];
        let mut cursor = WriteCursor::new(SliceSink::new(&mut output));
        write_value(&mut cursor, value).unwrap();
        assert_eq!(cursor.position(), length);
        assert_eq!(output, expected);
    }

    #[test]
    fn every_object_kind_encodes_an_exact_registry_checked_object() {
        for kind in PortableObjectKind::ALL {
            let input = fixture_object(kind, false);
            let length = measure_object(input, FormatLimits::HARD).unwrap();
            let mut output = vec![0xa5; usize::try_from(length).unwrap()];
            encode_object(input, FormatLimits::HARD, &mut output).unwrap();

            assert_eq!(&output[..4], &kind.magic());
            assert_eq!(
                u64::from_le_bytes(output[24..32].try_into().unwrap()),
                length
            );
            assert_eq!(output.len() as u64, length);
            let view = preflight_object_registry(&output, kind, FormatLimits::HARD).unwrap();
            assert_eq!(view.kind(), kind);
        }
    }

    #[test]
    fn prepared_object_reuses_one_preflight_and_preserves_atomic_length_failure() {
        let input = fixture_object(PortableObjectKind::CanonicalArtifact, true);
        let prepared = prepare_object(input, FormatLimits::HARD).unwrap();
        assert_eq!(
            prepared.byte_len(),
            measure_object(input, FormatLimits::HARD).unwrap()
        );

        let mut output = vec![0; usize::try_from(prepared.byte_len()).unwrap()];
        encode_prepared_object(prepared, &mut output).unwrap();
        preflight_object_registry(
            &output,
            PortableObjectKind::CanonicalArtifact,
            FormatLimits::HARD,
        )
        .unwrap();

        let mut short = vec![0x5a; output.len() - 1];
        let before = short.clone();
        assert_eq!(
            encode_prepared_object(prepared, &mut short)
                .unwrap_err()
                .class(),
            FormatErrorClass::LengthMismatch
        );
        assert_eq!(short, before);
    }

    #[cfg(feature = "std")]
    #[test]
    fn staged_writer_matches_slice_bytes_and_cleans_up_after_drop() {
        use std::{
            fs,
            sync::atomic::{AtomicU64, Ordering},
        };

        static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

        let input = fixture_object(PortableObjectKind::CanonicalArtifact, true);
        let prepared = prepare_object(input, FormatLimits::HARD).unwrap();
        let mut expected = vec![0; usize::try_from(prepared.byte_len()).unwrap()];
        encode_prepared_object(prepared, &mut expected).unwrap();

        let directory = std::env::temp_dir().join(std::format!(
            "laneflow-format-staged-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let source = crate::StagedObjectWriter::create_in(&directory, prepared)
            .unwrap()
            .finish()
            .unwrap();

        assert_eq!(source.exact_byte_length().get(), prepared.byte_len());
        assert_eq!(source.as_bytes().unwrap(), expected);
        assert_eq!(source.as_bytes().unwrap(), expected);
        drop(source);

        assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn lfcp_minimal_structural_input_has_frozen_exact_header_and_first_section_bytes() {
        let input = fixture_object(PortableObjectKind::CanonicalPublicationDescriptor, false);
        let length = measure_object(input, FormatLimits::HARD).unwrap();
        assert_eq!(length, 533);
        let mut output = vec![0; usize::try_from(length).unwrap()];
        encode_object(input, FormatLimits::HARD, &mut output).unwrap();

        let mut expected_header = Vec::new();
        expected_header.extend_from_slice(b"LFCP");
        expected_header.extend_from_slice(&2_u16.to_le_bytes());
        expected_header.extend_from_slice(&32_u16.to_le_bytes());
        expected_header.extend_from_slice(&0_u32.to_le_bytes());
        expected_header.extend_from_slice(&3_u32.to_le_bytes());
        expected_header.extend_from_slice(&32_u64.to_le_bytes());
        expected_header.extend_from_slice(&533_u64.to_le_bytes());
        for (kind, offset, section_length) in
            [(1_u16, 104_u64, 168_u64), (2, 272, 180), (3, 452, 81)]
        {
            expected_header.extend_from_slice(&kind.to_le_bytes());
            expected_header.extend_from_slice(&1_u16.to_le_bytes());
            expected_header.extend_from_slice(&0_u32.to_le_bytes());
            expected_header.extend_from_slice(&offset.to_le_bytes());
            expected_header.extend_from_slice(&section_length.to_le_bytes());
        }
        assert_eq!(&output[..104], expected_header);

        let mut expected_section = Vec::new();
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
        assert_eq!(&output[104..272], expected_section);
    }

    #[test]
    fn caller_object_budget_is_independent_of_the_chunk_ceiling() {
        let input = fixture_object(PortableObjectKind::CanonicalPublicationDescriptor, false);
        let exact_length = measure_object(input, FormatLimits::HARD).unwrap();

        let mut exact = FormatLimitConfig::HARD;
        exact.max_object_bytes = exact_length;
        let exact_limits = FormatLimits::try_new(exact).unwrap();
        let mut bytes = vec![0; usize::try_from(exact_length).unwrap()];
        encode_object(input, exact_limits, &mut bytes).unwrap();
        preflight_object_registry(
            &bytes,
            PortableObjectKind::CanonicalPublicationDescriptor,
            exact_limits,
        )
        .unwrap();

        let mut short = exact;
        short.max_object_bytes -= 1;
        assert!(matches!(
            measure_object(input, FormatLimits::try_new(short).unwrap()),
            Err(FormatError::LimitExceeded {
                dimension: LimitDimension::ObjectBytes,
                actual,
                limit,
            }) if actual == exact_length && limit == exact_length - 1
        ));
    }

    #[test]
    fn validation_and_output_length_failures_leave_the_destination_unchanged() {
        let valid = fixture_object(PortableObjectKind::CanonicalPublicationDescriptor, false);
        let length = measure_object(valid, FormatLimits::HARD).unwrap();
        let mut output = vec![0xa5; usize::try_from(length).unwrap()];
        let before = output.clone();

        let invalid = replace_first_value(valid, FieldWriteValue::U32(0));
        assert_eq!(
            encode_object(invalid, FormatLimits::HARD, &mut output)
                .unwrap_err()
                .class(),
            FormatErrorClass::BindingMismatch
        );
        assert_eq!(output, before);

        let mut short = vec![0x5a; usize::try_from(length - 1).unwrap()];
        let short_before = short.clone();
        assert_eq!(
            encode_object(valid, FormatLimits::HARD, &mut short)
                .unwrap_err()
                .class(),
            FormatErrorClass::LengthMismatch
        );
        assert_eq!(short, short_before);
    }

    #[test]
    fn nested_records_use_exact_counts_and_writer_limits_before_output() {
        let input = fixture_object(PortableObjectKind::CanonicalArtifact, true);
        let length = measure_object(input, FormatLimits::HARD).unwrap();
        let mut output = vec![0; usize::try_from(length).unwrap()];
        encode_object(input, FormatLimits::HARD, &mut output).unwrap();
        let view = preflight_object_registry(
            &output,
            PortableObjectKind::CanonicalArtifact,
            FormatLimits::HARD,
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

        let mut config = FormatLimitConfig::HARD;
        config.max_vector_items = 0;
        let limits = FormatLimits::try_new(config).unwrap();
        let mut untouched = vec![0x3c; usize::try_from(length).unwrap()];
        let before = untouched.clone();
        assert_eq!(
            encode_object(input, limits, &mut untouched)
                .unwrap_err()
                .class(),
            FormatErrorClass::LimitExceeded
        );
        assert_eq!(untouched, before);
    }

    #[test]
    fn writer_enforces_each_applicable_caller_budget_before_output() {
        let lfcp = fixture_object(PortableObjectKind::CanonicalPublicationDescriptor, false);
        let lfcp_length = measure_object(lfcp, FormatLimits::HARD).unwrap();

        let mut config = FormatLimitConfig::HARD;
        config.max_object_bytes = lfcp_length - 1;
        assert_limit_failure_is_atomic(lfcp, config, LimitDimension::ObjectBytes);

        let mut config = FormatLimitConfig::HARD;
        config.max_table_chunk_bytes = 100;
        assert_limit_failure_is_atomic(lfcp, config, LimitDimension::TableChunkBytes);

        let mut config = FormatLimitConfig::HARD;
        config.max_fields_per_row = 0;
        assert_limit_failure_is_atomic(lfcp, config, LimitDimension::FieldsPerRow);

        let lfcp_with_text = replace_top_level_value(lfcp, 1, 0, 0, 3, FieldWriteValue::Utf8("x"));
        let mut config = FormatLimitConfig::HARD;
        config.max_utf8_field_bytes = 0;
        assert_limit_failure_is_atomic(lfcp_with_text, config, LimitDimension::Utf8FieldBytes);

        let mut config = FormatLimitConfig::HARD;
        config.max_total_utf8_bytes = 0;
        assert_limit_failure_is_atomic(lfcp_with_text, config, LimitDimension::TotalUtf8Bytes);

        let lfca_with_record = fixture_object(PortableObjectKind::CanonicalArtifact, true);
        let mut config = FormatLimitConfig::HARD;
        config.max_vector_items = 0;
        assert_limit_failure_is_atomic(lfca_with_record, config, LimitDimension::VectorItems);

        let mut config = FormatLimitConfig::HARD;
        config.max_total_vector_bytes = 0;
        assert_limit_failure_is_atomic(lfca_with_record, config, LimitDimension::TotalVectorBytes);

        let mut config = FormatLimitConfig::HARD;
        config.max_record_vector_depth = 0;
        assert_limit_failure_is_atomic(lfca_with_record, config, LimitDimension::RecordVectorDepth);
    }

    #[test]
    fn caller_chunk_budgets_reject_without_changing_canonical_bytes() {
        let kind = PortableObjectKind::CanonicalArtifact;
        let schema = portable_object_schema(kind);
        let mut row = default_row(schema.sections[2].tables[1].row, false);
        let mut fields = row.fields.to_vec();
        fields[3].value = FieldWriteValue::Utf8("kind");
        fields[4].value = FieldWriteValue::OrdinalVectorU32(&[]);
        row.fields = leak(fields);
        let input = replace_table_rows(fixture_object(kind, false), 2, 1, leak(vec![row; 5]));
        let (row_length, row_budget) = encoded_row_measure(row);
        let canonical_chunk_length = TABLE_HEADER_BYTES + row_length * 5;
        assert_eq!(row_budget.total_utf8_bytes, 4);
        assert_eq!(row_budget.total_vector_bytes, 4);

        let hard_length = measure_object(input, FormatLimits::HARD).unwrap();
        let mut hard_bytes = vec![0; usize::try_from(hard_length).unwrap()];
        encode_object(input, FormatLimits::HARD, &mut hard_bytes).unwrap();

        let mut exact = FormatLimitConfig::HARD;
        exact.max_rows_per_chunk = 5;
        exact.max_table_chunk_bytes = canonical_chunk_length;
        exact.max_total_utf8_bytes = 20;
        exact.max_total_vector_bytes = 20;
        let exact_limits = FormatLimits::try_new(exact).unwrap();
        assert_eq!(measure_object(input, exact_limits).unwrap(), hard_length);
        let mut exact_bytes = vec![0; usize::try_from(hard_length).unwrap()];
        encode_object(input, exact_limits, &mut exact_bytes).unwrap();
        assert_eq!(exact_bytes, hard_bytes);

        let mut below = exact;
        below.max_rows_per_chunk = 4;
        assert_limit_failure_is_atomic(input, below, LimitDimension::RowsPerChunk);
        let mut below = exact;
        below.max_table_chunk_bytes = canonical_chunk_length - 1;
        assert_limit_failure_is_atomic(input, below, LimitDimension::TableChunkBytes);
        let mut below = exact;
        below.max_total_utf8_bytes = 19;
        assert_limit_failure_is_atomic(input, below, LimitDimension::TotalUtf8Bytes);
        let mut below = exact;
        below.max_total_vector_bytes = 19;
        assert_limit_failure_is_atomic(input, below, LimitDimension::TotalVectorBytes);
    }

    #[test]
    fn source_location_caller_budget_rejects_without_rechunking() {
        let kind = PortableObjectKind::SourceMap;
        let schema = portable_object_schema(kind);
        let row = default_row(schema.sections[1].tables[2].row, false);
        let input = replace_table_rows(fixture_object(kind, false), 1, 2, leak(vec![row; 5]));
        let hard_length = measure_object(input, FormatLimits::HARD).unwrap();
        let mut hard_bytes = vec![0; usize::try_from(hard_length).unwrap()];
        encode_object(input, FormatLimits::HARD, &mut hard_bytes).unwrap();

        let mut exact = FormatLimitConfig::HARD;
        exact.max_source_location_rows_per_chunk = 5;
        let exact_limits = FormatLimits::try_new(exact).unwrap();
        let mut exact_bytes = vec![0; usize::try_from(hard_length).unwrap()];
        encode_object(input, exact_limits, &mut exact_bytes).unwrap();
        assert_eq!(exact_bytes, hard_bytes);

        exact.max_source_location_rows_per_chunk = 4;
        assert_limit_failure_is_atomic(input, exact, LimitDimension::SourceLocationRowsPerChunk);
    }

    #[test]
    fn hard_row_ceiling_is_the_only_row_count_chunk_boundary() {
        let kind = PortableObjectKind::CanonicalArtifact;
        let schema = portable_object_schema(kind);
        let row = default_row(schema.sections[2].tables[0].row, false);
        let rows = leak(vec![row; FORMAT_HARD_MAX_ROWS_PER_CHUNK as usize + 1]);
        let table = TableWriteInput {
            kind: schema.sections[2].tables[0].kind,
            rows,
        };

        let (first_end, first_chunk) = next_chunk_end(kind, schema.sections[2].kind, table, 0);
        assert_eq!(first_end, FORMAT_HARD_MAX_ROWS_PER_CHUNK as usize);
        assert_eq!(first_chunk.row_count, FORMAT_HARD_MAX_ROWS_PER_CHUNK);
        let (second_end, second_chunk) =
            next_chunk_end(kind, schema.sections[2].kind, table, first_end);
        assert_eq!(second_end, rows.len());
        assert_eq!(second_chunk.row_count, 1);
    }

    #[test]
    fn prepared_section_cache_covers_the_closed_object_registry() {
        for kind in PortableObjectKind::ALL {
            assert!(kind.section_count() as usize <= MAX_PREPARED_SECTION_COUNT);
        }
    }

    #[test]
    fn chunk_directory_order_ranges_digests_and_budget_fail_closed() {
        let kind = PortableObjectKind::CanonicalArtifact;
        let schema = portable_object_schema(kind);
        let row = default_row(schema.sections[2].tables[0].row, false);
        let input = replace_table_rows(
            fixture_object(kind, false),
            2,
            0,
            leak(vec![row; FORMAT_HARD_MAX_ROWS_PER_CHUNK as usize + 1]),
        );
        let limits = FormatLimits::HARD;
        let length = measure_object(input, limits).unwrap();
        let mut original = vec![0; usize::try_from(length).unwrap()];
        encode_object(input, limits, &mut original).unwrap();
        let section_entry = usize::from(OBJECT_PREAMBLE_BYTE_LENGTH)
            + 2 * SECTION_DIRECTORY_ENTRY_BYTE_LENGTH as usize;
        let section_start = u64::from_le_bytes(
            original[section_entry + 8..section_entry + 16]
                .try_into()
                .unwrap(),
        ) as usize;
        let first = section_start + 16;
        let second = first + 72;
        assert_eq!(
            u32::from_le_bytes(
                original[section_start..section_start + 4]
                    .try_into()
                    .unwrap()
            ),
            2
        );
        assert_eq!(
            u32::from_le_bytes(original[first + 12..first + 16].try_into().unwrap()),
            FORMAT_HARD_MAX_ROWS_PER_CHUNK
        );
        assert_eq!(
            u32::from_le_bytes(original[second + 8..second + 12].try_into().unwrap()),
            FORMAT_HARD_MAX_ROWS_PER_CHUNK
        );
        assert_eq!(
            u32::from_le_bytes(original[second + 12..second + 16].try_into().unwrap()),
            1
        );

        let mut duplicate_index = original.clone();
        duplicate_index[second + 4..second + 8].copy_from_slice(&0_u32.to_le_bytes());
        assert_eq!(
            preflight_object_registry(&duplicate_index, kind, limits)
                .unwrap_err()
                .class(),
            FormatErrorClass::NonCanonicalOrder
        );

        let mut row_gap = original.clone();
        row_gap[second + 8..second + 12].copy_from_slice(&3_u32.to_le_bytes());
        assert_eq!(
            preflight_object_registry(&row_gap, kind, limits)
                .unwrap_err()
                .class(),
            FormatErrorClass::GapOrOverlap
        );

        let mut range_overlap = original.clone();
        let first_offset =
            u64::from_le_bytes(range_overlap[first + 24..first + 32].try_into().unwrap());
        range_overlap[second + 24..second + 32].copy_from_slice(&first_offset.to_le_bytes());
        assert_eq!(
            preflight_object_registry(&range_overlap, kind, limits)
                .unwrap_err()
                .class(),
            FormatErrorClass::GapOrOverlap
        );

        let mut digest = original.clone();
        digest[first + 40] ^= 1;
        assert_eq!(
            preflight_object_registry(&digest, kind, limits)
                .unwrap_err()
                .class(),
            FormatErrorClass::DigestMismatch
        );

        let mut payload = original;
        let payload_offset =
            u64::from_le_bytes(payload[first + 24..first + 32].try_into().unwrap()) as usize;
        payload[section_start + payload_offset] ^= 1;
        assert_eq!(
            preflight_object_registry(&payload, kind, limits)
                .unwrap_err()
                .class(),
            FormatErrorClass::DigestMismatch
        );

        let mut too_few_chunks = FormatLimitConfig::HARD;
        too_few_chunks.max_chunks_per_section = 1;
        assert!(matches!(
            measure_object(input, FormatLimits::try_new(too_few_chunks).unwrap()),
            Err(FormatError::LimitExceeded {
                dimension: LimitDimension::ChunksPerSection,
                actual: 2,
                limit: 1,
            })
        ));
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
                    FieldWriteValue::F32(value),
                    float_schema,
                    0,
                    FormatLimits::HARD,
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
        let mut config = FormatLimitConfig::HARD;
        config.max_total_utf8_bytes = 3;
        let limits = FormatLimits::try_new(config).unwrap();
        let mut budget = WriteBudget::default();
        assert_eq!(
            measure_value(
                FieldWriteValue::Utf8("ab"),
                utf8_schema,
                0,
                limits,
                &mut budget,
            ),
            Ok(2)
        );
        assert_eq!(
            measure_value(
                FieldWriteValue::Utf8("cd"),
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
    fn record_vector_budget_fails_before_visiting_later_rows() {
        const NESTED_FIELDS: &[PortableFieldSchema] = &[];
        const NESTED_ROW: PortableRowSchema = PortableRowSchema {
            fields: NESTED_FIELDS,
            shape: PortableRowShape::Uniform,
        };
        let schema = PortableFieldSchema {
            tag: 1,
            name: "records",
            field_type: PortableFieldType::RecordVector,
            presence: PortableFieldPresence::Required,
            nested_row: Some(&NESTED_ROW),
        };
        let invalid_fields = [FieldWriteInput {
            tag: 1,
            value: FieldWriteValue::U8(0),
        }];
        let rows = [
            RowWriteInput { fields: &[] },
            RowWriteInput {
                fields: &invalid_fields,
            },
        ];
        let mut config = FormatLimitConfig::HARD;
        config.max_total_vector_bytes = 4;
        let limits = FormatLimits::try_new(config).unwrap();

        assert_eq!(
            measure_value(
                FieldWriteValue::RecordVector(&rows),
                schema,
                0,
                limits,
                &mut WriteBudget::default(),
            ),
            Err(FormatError::LimitExceeded {
                dimension: LimitDimension::TotalVectorBytes,
                actual: 20,
                limit: 4,
            })
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
            FieldWriteValue::U8(0x12),
            schema(PortableFieldType::U8, None),
            &[0x12],
        );
        assert_value_bytes(
            FieldWriteValue::U16(0x1234),
            schema(PortableFieldType::U16, None),
            &[0x34, 0x12],
        );
        assert_value_bytes(
            FieldWriteValue::U32(0x1234_5678),
            schema(PortableFieldType::U32, None),
            &[0x78, 0x56, 0x34, 0x12],
        );
        assert_value_bytes(
            FieldWriteValue::U64(0x0123_4567_89ab_cdef),
            schema(PortableFieldType::U64, None),
            &0x0123_4567_89ab_cdef_u64.to_le_bytes(),
        );
        assert_value_bytes(
            FieldWriteValue::F32(1.5),
            schema(PortableFieldType::F32, None),
            &1.5_f32.to_bits().to_le_bytes(),
        );
        assert_value_bytes(
            FieldWriteValue::F64(-1.5),
            schema(PortableFieldType::F64, None),
            &(-1.5_f64).to_bits().to_le_bytes(),
        );
        assert_value_bytes(
            FieldWriteValue::StableId128([0x5a; 16]),
            schema(PortableFieldType::StableId128, None),
            &[0x5a; 16],
        );
        assert_value_bytes(
            FieldWriteValue::Sha256([0xa5; 32]),
            schema(PortableFieldType::Sha256, None),
            &[0xa5; 32],
        );
        assert_value_bytes(
            FieldWriteValue::Utf8("路"),
            schema(PortableFieldType::Utf8, None),
            "路".as_bytes(),
        );
        assert_value_bytes(
            FieldWriteValue::Bytes(&[0xde, 0xad]),
            schema(PortableFieldType::Bytes, None),
            &[0xde, 0xad],
        );
        assert_value_bytes(
            FieldWriteValue::OrdinalVectorU32(&[1, 0x1234_5678]),
            schema(PortableFieldType::OrdinalVectorU32, None),
            &[2, 0, 0, 0, 1, 0, 0, 0, 0x78, 0x56, 0x34, 0x12],
        );

        let nested_fields = leak(vec![FieldWriteInput {
            tag: 1,
            value: FieldWriteValue::U32(0x1234_5678),
        }]);
        let nested_rows = leak(vec![RowWriteInput {
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
            FieldWriteValue::RecordVector(nested_rows),
            schema(PortableFieldType::RecordVector, Some(&NESTED_ROW)),
            &nested_expected,
        );
        assert_value_bytes(
            FieldWriteValue::I32(-2),
            schema(PortableFieldType::I32, None),
            &(-2_i32).to_le_bytes(),
        );
    }
}
