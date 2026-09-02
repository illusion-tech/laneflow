//! TableV1 / RowV1 / FieldV1 的通用结构预检。
//!
//! 公共入口只执行所有 registry 都共同遵守的线格式约束。对象级入口另外传入附录 A 静态
//! registry，补齐 table kind、字段 tag/type/presence 和内嵌行形状；行键与跨表语义仍属于
//! 后继验证层。

use laneflow_static_contract::{
    PortableFieldPresence, PortableFieldSchema, PortableFieldType, PortableRowCardinality,
    PortableRowSchema, PortableRowShape, PortableTableSchema, portable_field_mask,
};

use crate::{
    FormatError, FormatLimits, FormatStructure, LimitDimension,
    limits::CanonicalRowMetrics,
    wire::{checked_slice, checked_slice_within, read_u8, read_u16, read_u32, read_u64},
};

const TABLE_HEADER_BYTES: u64 = 16;
const ROW_HEADER_BYTES: u64 = 16;
const FIELD_HEADER_BYTES: u64 = 12;
const TABLE_SCHEMA_VERSION: u16 = 1;

/// 一张 TableV1 完成通用结构预检后的计数摘要。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TableStructureSummary {
    table_kind: u16,
    top_level_rows: u32,
    total_rows: u64,
    total_fields: u64,
    total_utf8_bytes: u64,
    total_vector_bytes: u64,
    maximum_record_vector_depth: u8,
    first_row_exact_byte_length: u64,
    first_row_total_utf8_bytes: u64,
    first_row_total_vector_bytes: u64,
}

impl TableStructureSummary {
    #[must_use]
    pub const fn table_kind(self) -> u16 {
        self.table_kind
    }

    #[must_use]
    pub const fn top_level_rows(self) -> u32 {
        self.top_level_rows
    }

    #[must_use]
    pub const fn total_rows(self) -> u64 {
        self.total_rows
    }

    #[must_use]
    pub const fn total_fields(self) -> u64 {
        self.total_fields
    }

    #[must_use]
    pub const fn total_utf8_bytes(self) -> u64 {
        self.total_utf8_bytes
    }

    #[must_use]
    pub const fn total_vector_bytes(self) -> u64 {
        self.total_vector_bytes
    }

    #[must_use]
    pub const fn maximum_record_vector_depth(self) -> u8 {
        self.maximum_record_vector_depth
    }

    pub(crate) const fn first_row_metrics(self) -> CanonicalRowMetrics {
        CanonicalRowMetrics {
            exact_byte_length: self.first_row_exact_byte_length,
            total_utf8_bytes: self.first_row_total_utf8_bytes,
            total_vector_bytes: self.first_row_total_vector_bytes,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PreflightBudget {
    total_rows: u64,
    total_fields: u64,
    pub(crate) total_utf8_bytes: u64,
    pub(crate) total_vector_bytes: u64,
    maximum_record_vector_depth: u8,
}

pub(crate) fn preflight_embedded_row(
    bytes: &[u8],
    schema: &'static PortableRowSchema,
    limits: FormatLimits,
    budget: &mut PreflightBudget,
) -> Result<(), FormatError> {
    let end = parse_row(
        bytes,
        0,
        ContainerBoundary {
            end: bytes.len() as u64,
            structure: FormatStructure::Row,
        },
        0,
        Some(schema),
        limits,
        budget,
    )?;
    if end != bytes.len() as u64 {
        return Err(FormatError::LengthMismatch {
            structure: FormatStructure::Row,
            declared: end,
            actual: bytes.len() as u64,
        });
    }
    Ok(())
}

pub(crate) fn charge_stable_vector(
    bytes: u64,
    limits: FormatLimits,
    budget: &mut PreflightBudget,
) -> Result<(), FormatError> {
    precheck_variable_limits(
        PortableFieldType::OrdinalVectorU32,
        bytes,
        0,
        limits,
        budget,
    )
}

/// 对一张 exact TableV1 执行冗余长度、计数、字段和通用值结构预检。
pub fn preflight_table_structure(
    bytes: &[u8],
    expected_table_kind: u16,
    limits: FormatLimits,
) -> Result<TableStructureSummary, FormatError> {
    let mut budget = PreflightBudget::default();
    preflight_table_structure_with_registry(bytes, expected_table_kind, None, limits, &mut budget)
}

pub(crate) fn preflight_table_with_registry(
    bytes: &[u8],
    schema: &'static PortableTableSchema,
    limits: FormatLimits,
    budget: &mut PreflightBudget,
) -> Result<TableStructureSummary, FormatError> {
    preflight_table_structure_with_registry(bytes, schema.kind, Some(schema), limits, budget)
}

fn preflight_table_structure_with_registry(
    bytes: &[u8],
    expected_table_kind: u16,
    schema: Option<&'static PortableTableSchema>,
    limits: FormatLimits,
    budget: &mut PreflightBudget,
) -> Result<TableStructureSummary, FormatError> {
    let config = limits.config();
    let table_length = bytes.len() as u64;
    if table_length > config.max_table_chunk_bytes {
        return Err(FormatError::LimitExceeded {
            dimension: LimitDimension::TableChunkBytes,
            actual: table_length,
            limit: config.max_table_chunk_bytes,
        });
    }

    let table_kind = read_u16(bytes, 0, FormatStructure::Table)?;
    if table_kind == 0 {
        return Err(FormatError::UnknownKind {
            structure: FormatStructure::Table,
            code: 0,
        });
    }
    if table_kind != expected_table_kind {
        return Err(FormatError::BindingMismatch {
            structure: FormatStructure::Table,
        });
    }
    let schema_version = read_u16(bytes, 2, FormatStructure::Table)?;
    if schema_version != TABLE_SCHEMA_VERSION {
        return Err(FormatError::UnsupportedVersion {
            structure: FormatStructure::Table,
            actual: u64::from(schema_version),
            expected: u64::from(TABLE_SCHEMA_VERSION),
        });
    }
    let row_count = read_u32(bytes, 4, FormatStructure::Table)?;
    check_limit(
        LimitDimension::RowsPerChunk,
        u64::from(row_count),
        u64::from(config.max_rows_per_chunk),
    )?;
    if let Some(schema) = schema {
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
    }
    let rows_byte_length = read_u64(bytes, 8, FormatStructure::Table)?;
    let actual_rows_byte_length =
        table_length
            .checked_sub(TABLE_HEADER_BYTES)
            .ok_or(FormatError::Truncated {
                structure: FormatStructure::Table,
                offset: 0,
                needed: TABLE_HEADER_BYTES,
                available: table_length,
            })?;
    if rows_byte_length != actual_rows_byte_length {
        return Err(FormatError::LengthMismatch {
            structure: FormatStructure::TableRows,
            declared: rows_byte_length,
            actual: actual_rows_byte_length,
        });
    }
    let minimum_rows_bytes = u64::from(row_count).checked_mul(ROW_HEADER_BYTES).ok_or(
        FormatError::ArithmeticOverflow {
            structure: FormatStructure::TableRows,
        },
    )?;
    if minimum_rows_bytes > rows_byte_length {
        return Err(FormatError::LengthMismatch {
            structure: FormatStructure::TableRows,
            declared: u64::from(row_count),
            actual: rows_byte_length / ROW_HEADER_BYTES,
        });
    }

    let budget_before = *budget;
    let end = TABLE_HEADER_BYTES.checked_add(rows_byte_length).ok_or(
        FormatError::ArithmeticOverflow {
            structure: FormatStructure::Table,
        },
    )?;
    let boundary = ContainerBoundary {
        end,
        structure: FormatStructure::TableRows,
    };
    let (cursor, first_row_metrics) = if row_count == 0 {
        (
            TABLE_HEADER_BYTES,
            CanonicalRowMetrics {
                exact_byte_length: 0,
                total_utf8_bytes: 0,
                total_vector_bytes: 0,
            },
        )
    } else {
        let first_budget = *budget;
        let first_end = parse_row(
            bytes,
            TABLE_HEADER_BYTES,
            boundary,
            0,
            schema.map(|schema| schema.row),
            limits,
            budget,
        )?;
        let first_row_metrics = CanonicalRowMetrics {
            exact_byte_length: first_end - TABLE_HEADER_BYTES,
            total_utf8_bytes: budget.total_utf8_bytes - first_budget.total_utf8_bytes,
            total_vector_bytes: budget.total_vector_bytes - first_budget.total_vector_bytes,
        };
        let cursor = parse_rows(
            bytes,
            first_end,
            boundary,
            row_count - 1,
            0,
            schema.map(|schema| schema.row),
            limits,
            budget,
        )?;
        (cursor, first_row_metrics)
    };
    if cursor != end {
        return Err(FormatError::LengthMismatch {
            structure: FormatStructure::TableRows,
            declared: rows_byte_length,
            actual: cursor - TABLE_HEADER_BYTES,
        });
    }

    Ok(TableStructureSummary {
        table_kind,
        top_level_rows: row_count,
        total_rows: budget.total_rows - budget_before.total_rows,
        total_fields: budget.total_fields - budget_before.total_fields,
        total_utf8_bytes: budget.total_utf8_bytes - budget_before.total_utf8_bytes,
        total_vector_bytes: budget.total_vector_bytes - budget_before.total_vector_bytes,
        maximum_record_vector_depth: budget.maximum_record_vector_depth,
        first_row_exact_byte_length: first_row_metrics.exact_byte_length,
        first_row_total_utf8_bytes: first_row_metrics.total_utf8_bytes,
        first_row_total_vector_bytes: first_row_metrics.total_vector_bytes,
    })
}

#[cfg(test)]
fn preflight_table(
    bytes: &[u8],
    expected_table_kind: u16,
    limits: FormatLimits,
) -> Result<TableStructureSummary, FormatError> {
    preflight_table_structure(bytes, expected_table_kind, limits)
}

#[allow(clippy::too_many_arguments)]
fn parse_rows(
    bytes: &[u8],
    mut cursor: u64,
    boundary: ContainerBoundary,
    row_count: u32,
    depth: u8,
    row_schema: Option<&'static PortableRowSchema>,
    limits: FormatLimits,
    budget: &mut PreflightBudget,
) -> Result<u64, FormatError> {
    let available = boundary
        .end
        .checked_sub(cursor)
        .ok_or(FormatError::ArithmeticOverflow {
            structure: FormatStructure::Row,
        })?;
    let minimum = u64::from(row_count).checked_mul(ROW_HEADER_BYTES).ok_or(
        FormatError::ArithmeticOverflow {
            structure: FormatStructure::Row,
        },
    )?;
    if minimum > available {
        return Err(FormatError::LengthMismatch {
            structure: boundary.structure,
            declared: u64::from(row_count),
            actual: available / ROW_HEADER_BYTES,
        });
    }

    for _ in 0..row_count {
        cursor = parse_row(bytes, cursor, boundary, depth, row_schema, limits, budget)?;
    }
    Ok(cursor)
}

#[derive(Clone, Copy)]
struct ContainerBoundary {
    end: u64,
    structure: FormatStructure,
}

fn parse_row(
    bytes: &[u8],
    row_offset: u64,
    boundary: ContainerBoundary,
    depth: u8,
    row_schema: Option<&'static PortableRowSchema>,
    limits: FormatLimits,
    budget: &mut PreflightBudget,
) -> Result<u64, FormatError> {
    let config = limits.config();
    checked_slice_within(
        bytes,
        row_offset,
        ROW_HEADER_BYTES,
        boundary.end,
        boundary.structure,
    )?;
    let row_byte_length = read_u64(bytes, row_offset, FormatStructure::Row)?;
    if row_byte_length < ROW_HEADER_BYTES {
        return Err(FormatError::LengthMismatch {
            structure: FormatStructure::Row,
            declared: row_byte_length,
            actual: ROW_HEADER_BYTES,
        });
    }
    let row_end =
        row_offset
            .checked_add(row_byte_length)
            .ok_or(FormatError::ArithmeticOverflow {
                structure: FormatStructure::Row,
            })?;
    if row_end > boundary.end {
        return Err(FormatError::LengthMismatch {
            structure: FormatStructure::Row,
            declared: row_byte_length,
            actual: boundary.end.saturating_sub(row_offset),
        });
    }

    let field_count = read_u32(bytes, row_offset + 8, FormatStructure::Row)?;
    check_limit(
        LimitDimension::FieldsPerRow,
        u64::from(field_count),
        u64::from(config.max_fields_per_row),
    )?;
    let reserved = read_u32(bytes, row_offset + 12, FormatStructure::Row)?;
    if reserved != 0 {
        return Err(FormatError::NonCanonicalValue {
            structure: FormatStructure::Row,
            offset: row_offset + 12,
        });
    }
    let minimum_fields = u64::from(field_count)
        .checked_mul(FIELD_HEADER_BYTES)
        .ok_or(FormatError::ArithmeticOverflow {
            structure: FormatStructure::RowFields,
        })?;
    if minimum_fields > row_byte_length - ROW_HEADER_BYTES {
        return Err(FormatError::LengthMismatch {
            structure: FormatStructure::RowFields,
            declared: u64::from(field_count),
            actual: (row_byte_length - ROW_HEADER_BYTES) / FIELD_HEADER_BYTES,
        });
    }

    budget.total_rows =
        budget
            .total_rows
            .checked_add(1)
            .ok_or(FormatError::ArithmeticOverflow {
                structure: FormatStructure::Row,
            })?;
    budget.total_fields = budget
        .total_fields
        .checked_add(u64::from(field_count))
        .ok_or(FormatError::ArithmeticOverflow {
            structure: FormatStructure::RowFields,
        })?;

    let mut cursor = row_offset + ROW_HEADER_BYTES;
    let mut previous_tag = None;
    let mut schema_index = 0_usize;
    let mut seen_fields = 0_u32;
    let mut discriminant = None;
    for _ in 0..field_count {
        checked_slice_within(
            bytes,
            cursor,
            FIELD_HEADER_BYTES,
            row_end,
            FormatStructure::RowFields,
        )?;
        let actual_tag = read_u16(bytes, cursor, FormatStructure::Field)?;
        if actual_tag == 0 {
            return Err(FormatError::UnknownKind {
                structure: FormatStructure::Field,
                code: 0,
            });
        }
        if let Some(previous) = previous_tag
            && previous >= actual_tag
        {
            return Err(FormatError::NonCanonicalOrder {
                structure: FormatStructure::RowFields,
                previous: u64::from(previous),
                current: u64::from(actual_tag),
            });
        }
        previous_tag = Some(actual_tag);
        let expected_field = if let Some(schema) = row_schema {
            while schema_index < schema.fields.len() && schema.fields[schema_index].tag < actual_tag
            {
                schema_index += 1;
            }
            let expected =
                schema
                    .fields
                    .get(schema_index)
                    .copied()
                    .ok_or(FormatError::UnknownKind {
                        structure: FormatStructure::Field,
                        code: u64::from(actual_tag),
                    })?;
            if expected.tag != actual_tag {
                return Err(FormatError::UnknownKind {
                    structure: FormatStructure::Field,
                    code: u64::from(actual_tag),
                });
            }
            schema_index += 1;
            seen_fields |= portable_field_mask(actual_tag);
            Some(expected)
        } else {
            None
        };
        let parsed = parse_field(
            bytes,
            cursor,
            row_end,
            depth,
            expected_field,
            limits,
            budget,
        )?;
        if let Some(schema) = row_schema
            && let Some((tag, _)) = schema.shape.discriminant()
            && actual_tag == tag
        {
            discriminant = parsed.u8_value;
        }
        cursor = parsed.end;
    }
    if cursor != row_end {
        return Err(FormatError::LengthMismatch {
            structure: FormatStructure::RowFields,
            declared: row_byte_length - ROW_HEADER_BYTES,
            actual: cursor - row_offset - ROW_HEADER_BYTES,
        });
    }
    if let Some(schema) = row_schema {
        validate_row_shape(schema, seen_fields, discriminant)?;
        if schema.shape == PortableRowShape::PolicyLocalChange {
            crate::policy_value::preflight_change_values(
                checked_slice(bytes, row_offset, row_byte_length, FormatStructure::Row)?,
                limits,
                budget,
            )?;
        }
    }
    Ok(row_end)
}

#[derive(Clone, Copy, Debug)]
struct ParsedField {
    end: u64,
    u8_value: Option<u8>,
}

#[allow(clippy::too_many_arguments)]
fn parse_field(
    bytes: &[u8],
    field_offset: u64,
    row_end: u64,
    depth: u8,
    expected_field: Option<PortableFieldSchema>,
    limits: FormatLimits,
    budget: &mut PreflightBudget,
) -> Result<ParsedField, FormatError> {
    let field_type_code = read_u8(bytes, field_offset + 2, FormatStructure::Field)?;
    let field_type =
        PortableFieldType::from_code(field_type_code).ok_or(FormatError::UnknownKind {
            structure: FormatStructure::Field,
            code: u64::from(field_type_code),
        })?;
    if let Some(expected) = expected_field
        && field_type != expected.field_type
    {
        return Err(FormatError::BindingMismatch {
            structure: FormatStructure::Field,
        });
    }
    let flags = read_u8(bytes, field_offset + 3, FormatStructure::Field)?;
    if flags != 0 {
        return Err(FormatError::NonCanonicalValue {
            structure: FormatStructure::Field,
            offset: field_offset + 3,
        });
    }
    let value_byte_length = read_u64(bytes, field_offset + 4, FormatStructure::Field)?;
    if let Some(expected) = field_type.fixed_width()
        && value_byte_length != expected
    {
        return Err(FormatError::LengthMismatch {
            structure: FormatStructure::FieldValue,
            declared: value_byte_length,
            actual: expected,
        });
    }

    precheck_variable_limits(field_type, value_byte_length, depth, limits, budget)?;
    let value_offset =
        field_offset
            .checked_add(FIELD_HEADER_BYTES)
            .ok_or(FormatError::ArithmeticOverflow {
                structure: FormatStructure::Field,
            })?;
    let field_end =
        value_offset
            .checked_add(value_byte_length)
            .ok_or(FormatError::ArithmeticOverflow {
                structure: FormatStructure::Field,
            })?;
    if field_end > row_end {
        return Err(FormatError::LengthMismatch {
            structure: FormatStructure::FieldValue,
            declared: value_byte_length,
            actual: row_end.saturating_sub(value_offset),
        });
    }
    let value = checked_slice(
        bytes,
        value_offset,
        value_byte_length,
        FormatStructure::FieldValue,
    )?;

    match field_type {
        PortableFieldType::F32 => validate_f32(value, value_offset)?,
        PortableFieldType::F64 => validate_f64(value, value_offset)?,
        PortableFieldType::Utf8 => {
            if core::str::from_utf8(value).is_err() {
                return Err(FormatError::NonCanonicalValue {
                    structure: FormatStructure::FieldValue,
                    offset: value_offset,
                });
            }
        }
        PortableFieldType::OrdinalVectorU32 => {
            validate_ordinal_vector(bytes, value_offset, value_byte_length, limits)?;
        }
        PortableFieldType::RecordVector => {
            validate_record_vector(
                bytes,
                value_offset,
                value_byte_length,
                depth,
                expected_field.and_then(|field| field.nested_row),
                limits,
                budget,
            )?;
        }
        PortableFieldType::U8
        | PortableFieldType::U16
        | PortableFieldType::U32
        | PortableFieldType::U64
        | PortableFieldType::StableId128
        | PortableFieldType::Sha256
        | PortableFieldType::Bytes
        | PortableFieldType::I32 => {}
    }

    Ok(ParsedField {
        end: field_end,
        u8_value: (field_type == PortableFieldType::U8).then(|| value[0]),
    })
}

fn precheck_variable_limits(
    field_type: PortableFieldType,
    value_byte_length: u64,
    depth: u8,
    limits: FormatLimits,
    budget: &mut PreflightBudget,
) -> Result<(), FormatError> {
    let config = limits.config();
    match field_type {
        PortableFieldType::Utf8 => {
            check_limit(
                LimitDimension::Utf8FieldBytes,
                value_byte_length,
                config.max_utf8_field_bytes,
            )?;
            budget.total_utf8_bytes = budget
                .total_utf8_bytes
                .checked_add(value_byte_length)
                .ok_or(FormatError::ArithmeticOverflow {
                    structure: FormatStructure::FieldValue,
                })?;
            check_limit(
                LimitDimension::TotalUtf8Bytes,
                budget.total_utf8_bytes,
                config.max_total_utf8_bytes,
            )?;
        }
        PortableFieldType::OrdinalVectorU32 | PortableFieldType::RecordVector => {
            budget.total_vector_bytes = budget
                .total_vector_bytes
                .checked_add(value_byte_length)
                .ok_or(FormatError::ArithmeticOverflow {
                    structure: FormatStructure::FieldValue,
                })?;
            check_limit(
                LimitDimension::TotalVectorBytes,
                budget.total_vector_bytes,
                config.max_total_vector_bytes,
            )?;
            if field_type == PortableFieldType::RecordVector {
                let next_depth = depth
                    .checked_add(1)
                    .ok_or(FormatError::ArithmeticOverflow {
                        structure: FormatStructure::RecordVector,
                    })?;
                check_limit(
                    LimitDimension::RecordVectorDepth,
                    u64::from(next_depth),
                    u64::from(config.max_record_vector_depth),
                )?;
                budget.maximum_record_vector_depth =
                    budget.maximum_record_vector_depth.max(next_depth);
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_f32(value: &[u8], offset: u64) -> Result<(), FormatError> {
    let bits = u32::from_le_bytes(value.try_into().map_err(|_| FormatError::LengthMismatch {
        structure: FormatStructure::FieldValue,
        declared: value.len() as u64,
        actual: 4,
    })?);
    if bits == 0x8000_0000 || f32::from_bits(bits).is_nan() || f32::from_bits(bits).is_infinite() {
        return Err(FormatError::NonCanonicalValue {
            structure: FormatStructure::FieldValue,
            offset,
        });
    }
    Ok(())
}

fn validate_f64(value: &[u8], offset: u64) -> Result<(), FormatError> {
    let bits = u64::from_le_bytes(value.try_into().map_err(|_| FormatError::LengthMismatch {
        structure: FormatStructure::FieldValue,
        declared: value.len() as u64,
        actual: 8,
    })?);
    if bits == 0x8000_0000_0000_0000
        || f64::from_bits(bits).is_nan()
        || f64::from_bits(bits).is_infinite()
    {
        return Err(FormatError::NonCanonicalValue {
            structure: FormatStructure::FieldValue,
            offset,
        });
    }
    Ok(())
}

fn validate_ordinal_vector(
    bytes: &[u8],
    value_offset: u64,
    value_byte_length: u64,
    limits: FormatLimits,
) -> Result<(), FormatError> {
    if value_byte_length < 4 {
        return Err(FormatError::LengthMismatch {
            structure: FormatStructure::OrdinalVector,
            declared: value_byte_length,
            actual: 4,
        });
    }
    let count = read_u32(bytes, value_offset, FormatStructure::OrdinalVector)?;
    check_limit(
        LimitDimension::VectorItems,
        u64::from(count),
        u64::from(limits.config().max_vector_items),
    )?;
    let expected = u64::from(count)
        .checked_mul(4)
        .and_then(|length| length.checked_add(4))
        .ok_or(FormatError::ArithmeticOverflow {
            structure: FormatStructure::OrdinalVector,
        })?;
    if value_byte_length != expected {
        return Err(FormatError::LengthMismatch {
            structure: FormatStructure::OrdinalVector,
            declared: value_byte_length,
            actual: expected,
        });
    }
    Ok(())
}

fn validate_record_vector(
    bytes: &[u8],
    value_offset: u64,
    value_byte_length: u64,
    depth: u8,
    nested_row: Option<&'static PortableRowSchema>,
    limits: FormatLimits,
    budget: &mut PreflightBudget,
) -> Result<(), FormatError> {
    if value_byte_length < 4 {
        return Err(FormatError::LengthMismatch {
            structure: FormatStructure::RecordVector,
            declared: value_byte_length,
            actual: 4,
        });
    }
    let count = read_u32(bytes, value_offset, FormatStructure::RecordVector)?;
    check_limit(
        LimitDimension::VectorItems,
        u64::from(count),
        u64::from(limits.config().max_vector_items),
    )?;
    let rows_offset = value_offset + 4;
    let rows_end =
        value_offset
            .checked_add(value_byte_length)
            .ok_or(FormatError::ArithmeticOverflow {
                structure: FormatStructure::RecordVector,
            })?;
    let cursor = parse_rows(
        bytes,
        rows_offset,
        ContainerBoundary {
            end: rows_end,
            structure: FormatStructure::RecordVector,
        },
        count,
        depth + 1,
        nested_row,
        limits,
        budget,
    )?;
    if cursor != rows_end {
        return Err(FormatError::LengthMismatch {
            structure: FormatStructure::RecordVector,
            declared: value_byte_length - 4,
            actual: cursor - rows_offset,
        });
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use std::vec::Vec;

    use laneflow_static_contract::{
        FORMAT_HARD_MAX_FIELDS_PER_ROW, FORMAT_HARD_MAX_ROWS_PER_CHUNK,
        FORMAT_HARD_MAX_VECTOR_ITEMS,
    };

    use super::*;
    use crate::{FormatErrorClass, FormatLimitConfig};

    fn field(tag: u16, field_type: PortableFieldType, value: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&tag.to_le_bytes());
        bytes.push(field_type as u8);
        bytes.push(0);
        bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
        bytes.extend_from_slice(value);
        bytes
    }

    fn row(fields: &[Vec<u8>]) -> Vec<u8> {
        let length = 16 + fields.iter().map(Vec::len).sum::<usize>();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(length as u64).to_le_bytes());
        bytes.extend_from_slice(&(fields.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        for field in fields {
            bytes.extend_from_slice(field);
        }
        bytes
    }

    fn record_vector(rows: &[Vec<u8>]) -> Vec<u8> {
        let mut value = Vec::new();
        value.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        for row in rows {
            value.extend_from_slice(row);
        }
        value
    }

    fn table(kind: u16, rows: &[Vec<u8>]) -> Vec<u8> {
        let rows_length = rows.iter().map(Vec::len).sum::<usize>();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&kind.to_le_bytes());
        bytes.extend_from_slice(&TABLE_SCHEMA_VERSION.to_le_bytes());
        bytes.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(rows_length as u64).to_le_bytes());
        for row in rows {
            bytes.extend_from_slice(row);
        }
        bytes
    }

    fn valid_table() -> Vec<u8> {
        let nested = row(&[field(
            1,
            PortableFieldType::F32,
            &0_f32.to_bits().to_le_bytes(),
        )]);
        let ordinals = [2_u32, 7]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>();
        let mut ordinal_vector = Vec::from(2_u32.to_le_bytes());
        ordinal_vector.extend_from_slice(&ordinals);
        let outer = row(&[
            field(1, PortableFieldType::U32, &7_u32.to_le_bytes()),
            field(2, PortableFieldType::Utf8, b"abc"),
            field(3, PortableFieldType::OrdinalVectorU32, &ordinal_vector),
            field(
                4,
                PortableFieldType::RecordVector,
                &record_vector(&[nested]),
            ),
        ]);
        table(9, &[outer])
    }

    #[test]
    fn valid_nested_structure_reports_exact_budget_summary() {
        let bytes = valid_table();
        let summary = preflight_table(&bytes, 9, FormatLimits::HARD).unwrap();

        assert_eq!(summary.table_kind(), 9);
        assert_eq!(summary.top_level_rows(), 1);
        assert_eq!(summary.total_rows(), 2);
        assert_eq!(summary.total_fields(), 5);
        assert_eq!(summary.total_utf8_bytes(), 3);
        assert_eq!(summary.total_vector_bytes(), 48);
        assert_eq!(summary.maximum_record_vector_depth(), 1);
    }

    #[test]
    fn redundant_row_and_field_counts_fail_even_when_outer_lengths_are_self_consistent() {
        let original = valid_table();

        let mut bytes = original.clone();
        bytes[4..8].copy_from_slice(&0_u32.to_le_bytes());
        assert_eq!(
            preflight_table(&bytes, 9, FormatLimits::HARD)
                .unwrap_err()
                .class(),
            FormatErrorClass::LengthMismatch
        );

        let mut bytes = original;
        bytes[24..28].copy_from_slice(&0_u32.to_le_bytes());
        assert_eq!(
            preflight_table(&bytes, 9, FormatLimits::HARD)
                .unwrap_err()
                .class(),
            FormatErrorClass::LengthMismatch
        );
    }

    #[test]
    fn row_and_record_vector_headers_cannot_cross_their_declared_containers() {
        let mut first = row(&[field(1, PortableFieldType::Bytes, &[0; 12])]);
        first[8..12].copy_from_slice(&2_u32.to_le_bytes());
        let bytes = table(1, &[first, row(&[])]);
        assert_eq!(
            preflight_table(&bytes, 1, FormatLimits::HARD),
            Err(FormatError::LengthMismatch {
                structure: FormatStructure::RowFields,
                declared: FIELD_HEADER_BYTES,
                actual: 0,
            })
        );

        let nested = row(&[field(1, PortableFieldType::Bytes, &[0; 4])]);
        let mut vector = record_vector(&[nested]);
        vector[0..4].copy_from_slice(&2_u32.to_le_bytes());
        let outer = row(&[
            field(1, PortableFieldType::RecordVector, &vector),
            field(2, PortableFieldType::U8, &[0]),
        ]);
        let bytes = table(1, &[outer]);
        assert_eq!(
            preflight_table(&bytes, 1, FormatLimits::HARD),
            Err(FormatError::LengthMismatch {
                structure: FormatStructure::RecordVector,
                declared: ROW_HEADER_BYTES,
                actual: 0,
            })
        );
    }

    #[test]
    fn fixed_width_and_vector_redundant_lengths_fail_closed() {
        let wrong_fixed = table(1, &[row(&[field(1, PortableFieldType::U32, &[0_u8; 8])])]);
        assert_eq!(
            preflight_table(&wrong_fixed, 1, FormatLimits::HARD)
                .unwrap_err()
                .class(),
            FormatErrorClass::LengthMismatch
        );

        let mut vector = Vec::from(2_u32.to_le_bytes());
        vector.extend_from_slice(&7_u32.to_le_bytes());
        let wrong_vector = table(
            1,
            &[row(&[field(
                1,
                PortableFieldType::OrdinalVectorU32,
                &vector,
            )])],
        );
        assert_eq!(
            preflight_table(&wrong_vector, 1, FormatLimits::HARD)
                .unwrap_err()
                .class(),
            FormatErrorClass::LengthMismatch
        );
    }

    #[test]
    fn second_level_record_vector_is_rejected_before_nested_value_use() {
        let deepest = row(&[]);
        let inner = row(&[field(
            1,
            PortableFieldType::RecordVector,
            &record_vector(&[deepest]),
        )]);
        let outer = row(&[field(
            1,
            PortableFieldType::RecordVector,
            &record_vector(&[inner]),
        )]);
        let bytes = table(1, &[outer]);

        assert_eq!(
            preflight_table(&bytes, 1, FormatLimits::HARD),
            Err(FormatError::LimitExceeded {
                dimension: LimitDimension::RecordVectorDepth,
                actual: 2,
                limit: 1,
            })
        );
    }

    #[test]
    fn hard_count_limits_reject_plus_one_before_row_or_field_slices() {
        let mut excessive_rows = table(1, &[]);
        excessive_rows[4..8].copy_from_slice(&(FORMAT_HARD_MAX_ROWS_PER_CHUNK + 1).to_le_bytes());
        assert_eq!(
            preflight_table(&excessive_rows, 1, FormatLimits::HARD),
            Err(FormatError::LimitExceeded {
                dimension: LimitDimension::RowsPerChunk,
                actual: u64::from(FORMAT_HARD_MAX_ROWS_PER_CHUNK) + 1,
                limit: u64::from(FORMAT_HARD_MAX_ROWS_PER_CHUNK),
            })
        );

        let fields = (1..=FORMAT_HARD_MAX_FIELDS_PER_ROW + 1)
            .map(|tag| field(tag as u16, PortableFieldType::U8, &[0]))
            .collect::<Vec<_>>();
        let excessive_fields = table(1, &[row(&fields)]);
        assert_eq!(
            preflight_table(&excessive_fields, 1, FormatLimits::HARD),
            Err(FormatError::LimitExceeded {
                dimension: LimitDimension::FieldsPerRow,
                actual: u64::from(FORMAT_HARD_MAX_FIELDS_PER_ROW) + 1,
                limit: u64::from(FORMAT_HARD_MAX_FIELDS_PER_ROW),
            })
        );

        let vector = Vec::from((FORMAT_HARD_MAX_VECTOR_ITEMS + 1).to_le_bytes());
        let excessive_items = table(
            1,
            &[row(&[field(
                1,
                PortableFieldType::OrdinalVectorU32,
                &vector,
            )])],
        );
        assert_eq!(
            preflight_table(&excessive_items, 1, FormatLimits::HARD),
            Err(FormatError::LimitExceeded {
                dimension: LimitDimension::VectorItems,
                actual: u64::from(FORMAT_HARD_MAX_VECTOR_ITEMS) + 1,
                limit: u64::from(FORMAT_HARD_MAX_VECTOR_ITEMS),
            })
        );
    }

    #[test]
    fn caller_cumulative_budgets_are_checked_before_value_interpretation() {
        let bytes = valid_table();
        let mut config = FormatLimitConfig::HARD;
        config.max_total_utf8_bytes = 2;
        let limits = FormatLimits::try_new(config).unwrap();
        assert_eq!(
            preflight_table(&bytes, 9, limits),
            Err(FormatError::LimitExceeded {
                dimension: LimitDimension::TotalUtf8Bytes,
                actual: 3,
                limit: 2,
            })
        );

        let mut config = FormatLimitConfig::HARD;
        config.max_total_vector_bytes = 11;
        let limits = FormatLimits::try_new(config).unwrap();
        assert_eq!(
            preflight_table(&bytes, 9, limits),
            Err(FormatError::LimitExceeded {
                dimension: LimitDimension::TotalVectorBytes,
                actual: 12,
                limit: 11,
            })
        );
    }

    #[test]
    fn noncanonical_float_utf8_and_field_order_are_rejected() {
        for bits in [f32::NAN.to_bits(), f32::INFINITY.to_bits(), 0x8000_0000] {
            let bytes = table(
                1,
                &[row(&[field(
                    1,
                    PortableFieldType::F32,
                    &bits.to_le_bytes(),
                )])],
            );
            assert_eq!(
                preflight_table(&bytes, 1, FormatLimits::HARD)
                    .unwrap_err()
                    .class(),
                FormatErrorClass::NonCanonicalValue
            );
        }

        let invalid_utf8 = table(1, &[row(&[field(1, PortableFieldType::Utf8, &[0xff])])]);
        assert_eq!(
            preflight_table(&invalid_utf8, 1, FormatLimits::HARD)
                .unwrap_err()
                .class(),
            FormatErrorClass::NonCanonicalValue
        );

        let unordered = table(
            1,
            &[row(&[
                field(2, PortableFieldType::U8, &[0]),
                field(1, PortableFieldType::U8, &[0]),
            ])],
        );
        assert_eq!(
            preflight_table(&unordered, 1, FormatLimits::HARD)
                .unwrap_err()
                .class(),
            FormatErrorClass::NonCanonicalOrder
        );
    }

    #[test]
    fn caller_can_reject_table_before_structural_walk() {
        let bytes = valid_table();
        let mut config = FormatLimitConfig::HARD;
        config.max_table_chunk_bytes = bytes.len() as u64 - 1;
        let limits = FormatLimits::try_new(config).unwrap();

        assert_eq!(
            preflight_table(&bytes, 9, limits),
            Err(FormatError::LimitExceeded {
                dimension: LimitDimension::TableChunkBytes,
                actual: bytes.len() as u64,
                limit: bytes.len() as u64 - 1,
            })
        );
    }
}
