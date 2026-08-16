//! #298 `SEC-003/008..015` 的集中边界矩阵。
//!
//! 测试直接构造冻结的 Table/Row/Field wire primitives，并只调用生产 preflight；这里不
//! 复制对象 emitter 或建立第二套规范预言机。

use std::{vec, vec::Vec};

use laneflow_static_contract::{
    FORMAT_HARD_MAX_FIELDS_PER_ROW, FORMAT_HARD_MAX_OBJECT_BYTES, FORMAT_HARD_MAX_ROWS_PER_TABLE,
    FORMAT_HARD_MAX_SECTION_OR_TABLE_BYTES, FORMAT_HARD_MAX_SOURCE_LOCATION_ROWS,
    FORMAT_HARD_MAX_TOTAL_UTF8_BYTES, FORMAT_HARD_MAX_TOTAL_VECTOR_BYTES,
    FORMAT_HARD_MAX_UTF8_FIELD_BYTES, FORMAT_HARD_MAX_VECTOR_ITEMS, OBJECT_PREAMBLE_V1_BYTE_LENGTH,
    PortableFieldType, PortableObjectKind, SECTION_DIRECTORY_ENTRY_V1_BYTE_LENGTH,
    SECTION_FORMAT_VERSION_V1, portable_object_schema,
};

use crate::{
    FormatError, FormatErrorClass, FormatLimits, FormatStructure, LimitDimension,
    preflight_object_framing, preflight_table_structure_v1,
    table::{PreflightBudget, preflight_table_with_registry_v1},
    wire::checked_slice,
};

const TABLE_HEADER_BYTES: usize = 16;
const ROW_HEADER_BYTES: usize = 16;
const FIELD_HEADER_BYTES: usize = 12;

fn field(tag: u16, field_type: PortableFieldType, value: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(FIELD_HEADER_BYTES + value.len());
    bytes.extend_from_slice(&tag.to_le_bytes());
    bytes.push(field_type as u8);
    bytes.push(0);
    bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
    bytes.extend_from_slice(value);
    bytes
}

fn row(fields: &[Vec<u8>]) -> Vec<u8> {
    let length = ROW_HEADER_BYTES + fields.iter().map(Vec::len).sum::<usize>();
    let mut bytes = Vec::with_capacity(length);
    bytes.extend_from_slice(&(length as u64).to_le_bytes());
    bytes.extend_from_slice(&(fields.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    for field in fields {
        bytes.extend_from_slice(field);
    }
    bytes
}

fn table(kind: u16, rows: &[Vec<u8>]) -> Vec<u8> {
    let rows_length = rows.iter().map(Vec::len).sum::<usize>();
    let mut bytes = Vec::with_capacity(TABLE_HEADER_BYTES + rows_length);
    bytes.extend_from_slice(&kind.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&(rows_length as u64).to_le_bytes());
    for row in rows {
        bytes.extend_from_slice(row);
    }
    bytes
}

fn object_with_section_lengths(kind: PortableObjectKind, section_lengths: &[u64]) -> Vec<u8> {
    assert_eq!(section_lengths.len(), kind.section_count() as usize);
    let total = section_lengths
        .iter()
        .copied()
        .try_fold(kind.first_section_offset(), u64::checked_add)
        .unwrap();
    let mut bytes = vec![0_u8; usize::try_from(total).unwrap()];
    bytes[0..4].copy_from_slice(&kind.magic());
    bytes[4..6].copy_from_slice(&kind.format_version().to_le_bytes());
    bytes[6..8].copy_from_slice(&OBJECT_PREAMBLE_V1_BYTE_LENGTH.to_le_bytes());
    bytes[12..16].copy_from_slice(&kind.section_count().to_le_bytes());
    bytes[16..24].copy_from_slice(&u64::from(OBJECT_PREAMBLE_V1_BYTE_LENGTH).to_le_bytes());
    bytes[24..32].copy_from_slice(&total.to_le_bytes());

    let mut section_offset = kind.first_section_offset();
    for (ordinal, byte_length) in section_lengths.iter().copied().enumerate() {
        let entry = usize::from(OBJECT_PREAMBLE_V1_BYTE_LENGTH)
            + ordinal * usize::try_from(SECTION_DIRECTORY_ENTRY_V1_BYTE_LENGTH).unwrap();
        bytes[entry..entry + 2].copy_from_slice(&u16::try_from(ordinal + 1).unwrap().to_le_bytes());
        bytes[entry + 2..entry + 4].copy_from_slice(&SECTION_FORMAT_VERSION_V1.to_le_bytes());
        bytes[entry + 8..entry + 16].copy_from_slice(&section_offset.to_le_bytes());
        bytes[entry + 16..entry + 24].copy_from_slice(&byte_length.to_le_bytes());
        section_offset = section_offset.checked_add(byte_length).unwrap();
    }
    bytes
}

fn ordinal_vector(count: u32, trailing_bytes: usize) -> Vec<u8> {
    let mut value = Vec::with_capacity(4 + count as usize * 4 + trailing_bytes);
    value.extend_from_slice(&count.to_le_bytes());
    value.resize(4 + count as usize * 4 + trailing_bytes, 0);
    value
}

fn vector_budget_table(last_count: u32, last_trailing_bytes: usize) -> Vec<u8> {
    let mut rows = Vec::new();
    let mut vector_index = 0_u32;
    for _ in 0..2 {
        let mut fields = Vec::new();
        for tag in 1..=16_u16 {
            let is_last = vector_index == 31;
            let count = if is_last {
                last_count
            } else {
                FORMAT_HARD_MAX_VECTOR_ITEMS
            };
            fields.push(field(
                tag,
                PortableFieldType::OrdinalVectorU32,
                &ordinal_vector(count, usize::from(is_last) * last_trailing_bytes),
            ));
            vector_index += 1;
        }
        rows.push(row(&fields));
    }
    table(1, &rows)
}

#[test]
fn checked_wire_offsets_fail_with_arithmetic_overflow() {
    assert_eq!(
        checked_slice(&[], u64::MAX, 1, FormatStructure::FieldValue),
        Err(FormatError::ArithmeticOverflow {
            structure: FormatStructure::FieldValue,
        })
    );
}

#[test]
fn redundant_row_and_field_counts_reject_both_minus_and_plus_one() {
    let original = table(1, &[row(&[field(1, PortableFieldType::U8, &[0])])]);
    for count in [0_u32, 2] {
        let mut bytes = original.clone();
        bytes[4..8].copy_from_slice(&count.to_le_bytes());
        assert_eq!(
            preflight_table_structure_v1(&bytes, 1, FormatLimits::V1_HARD)
                .unwrap_err()
                .class(),
            FormatErrorClass::LengthMismatch
        );

        let mut bytes = original.clone();
        bytes[24..28].copy_from_slice(&count.to_le_bytes());
        assert_eq!(
            preflight_table_structure_v1(&bytes, 1, FormatLimits::V1_HARD)
                .unwrap_err()
                .class(),
            FormatErrorClass::LengthMismatch
        );
    }
}

#[test]
fn all_fixed_widths_and_both_vector_counts_reject_redundant_mismatch() {
    let fixed_widths = [
        (PortableFieldType::U8, 1_usize),
        (PortableFieldType::U16, 2),
        (PortableFieldType::U32, 4),
        (PortableFieldType::U64, 8),
        (PortableFieldType::F32, 4),
        (PortableFieldType::F64, 8),
        (PortableFieldType::StableId128, 16),
        (PortableFieldType::Sha256, 32),
        (PortableFieldType::I32, 4),
    ];
    for (field_type, width) in fixed_widths {
        let bytes = table(1, &[row(&[field(1, field_type, &vec![0; width + 1])])]);
        assert_eq!(
            preflight_table_structure_v1(&bytes, 1, FormatLimits::V1_HARD)
                .unwrap_err()
                .class(),
            FormatErrorClass::LengthMismatch,
            "{field_type:?} accepted a wrong VBL"
        );
    }

    let ordinal = table(
        1,
        &[row(&[field(
            1,
            PortableFieldType::OrdinalVectorU32,
            &[2_u32.to_le_bytes(), 0_u32.to_le_bytes()].concat(),
        )])],
    );
    assert_eq!(
        preflight_table_structure_v1(&ordinal, 1, FormatLimits::V1_HARD)
            .unwrap_err()
            .class(),
        FormatErrorClass::LengthMismatch
    );

    let nested = row(&[]);
    let record_value = [2_u32.to_le_bytes().as_slice(), nested.as_slice()].concat();
    let record = table(
        1,
        &[row(&[field(
            1,
            PortableFieldType::RecordVector,
            &record_value,
        )])],
    );
    assert_eq!(
        preflight_table_structure_v1(&record, 1, FormatLimits::V1_HARD)
            .unwrap_err()
            .class(),
        FormatErrorClass::LengthMismatch
    );
}

#[test]
fn every_redundant_outer_length_layer_fails_closed() {
    let kind = PortableObjectKind::CanonicalPublicationDescriptor;
    let original_object = object_with_section_lengths(kind, &[4, 4, 4, 4]);

    let mut object_length = original_object.clone();
    let wrong_object_length = object_length.len() as u64 + 1;
    object_length[24..32].copy_from_slice(&wrong_object_length.to_le_bytes());
    assert!(preflight_object_framing(&object_length, kind, FormatLimits::V1_HARD).is_err());

    let mut section_length = original_object;
    let first_entry = usize::from(OBJECT_PREAMBLE_V1_BYTE_LENGTH);
    section_length[first_entry + 16..first_entry + 24].copy_from_slice(&5_u64.to_le_bytes());
    assert!(preflight_object_framing(&section_length, kind, FormatLimits::V1_HARD).is_err());

    let original_table = table(1, &[row(&[field(1, PortableFieldType::Bytes, &[0])])]);
    for range in [8..16, 16..24, 36..44] {
        let mut bytes = original_table.clone();
        let current = u64::from_le_bytes(bytes[range.clone()].try_into().unwrap());
        bytes[range].copy_from_slice(&(current + 1).to_le_bytes());
        assert!(
            preflight_table_structure_v1(&bytes, 1, FormatLimits::V1_HARD).is_err(),
            "length field at offset accepted an inconsistent inner total"
        );
    }
}

#[test]
fn every_table_byte_boundary_truncates_with_a_stable_length_class() {
    let original = table(
        1,
        &[row(&[
            field(1, PortableFieldType::U32, &7_u32.to_le_bytes()),
            field(2, PortableFieldType::Utf8, b"boundary"),
        ])],
    );
    for boundary in 0..original.len() {
        let mut truncated = original[..boundary].to_vec();
        if boundary >= TABLE_HEADER_BYTES {
            truncated[8..16].copy_from_slice(
                &u64::try_from(boundary - TABLE_HEADER_BYTES)
                    .unwrap()
                    .to_le_bytes(),
            );
        }
        if boundary >= TABLE_HEADER_BYTES + ROW_HEADER_BYTES {
            truncated[16..24].copy_from_slice(
                &u64::try_from(boundary - TABLE_HEADER_BYTES)
                    .unwrap()
                    .to_le_bytes(),
            );
        }
        let class = preflight_table_structure_v1(&truncated, 1, FormatLimits::V1_HARD)
            .unwrap_err()
            .class();
        assert!(
            matches!(
                class,
                FormatErrorClass::Truncated | FormatErrorClass::LengthMismatch
            ),
            "boundary {boundary} returned {class:?}"
        );
    }
}

#[test]
fn object_and_table_byte_boundaries_have_explicit_reachability() {
    let kind = PortableObjectKind::CanonicalPublicationDescriptor;
    let first_section_length = FORMAT_HARD_MAX_OBJECT_BYTES - kind.first_section_offset();
    let mut object = object_with_section_lengths(kind, &[first_section_length, 0, 0, 0]);
    assert_eq!(object.len() as u64, FORMAT_HARD_MAX_OBJECT_BYTES);
    preflight_object_framing(&object, kind, FormatLimits::V1_HARD).unwrap();
    object.push(0);
    let over_object_length = object.len() as u64;
    object[24..32].copy_from_slice(&over_object_length.to_le_bytes());
    let first_entry = usize::from(OBJECT_PREAMBLE_V1_BYTE_LENGTH);
    object[first_entry + 16..first_entry + 24]
        .copy_from_slice(&(first_section_length + 1).to_le_bytes());
    assert_eq!(
        preflight_object_framing(&object, kind, FormatLimits::V1_HARD).unwrap_err(),
        FormatError::LimitExceeded {
            dimension: LimitDimension::ObjectBytes,
            actual: FORMAT_HARD_MAX_OBJECT_BYTES + 1,
            limit: FORMAT_HARD_MAX_OBJECT_BYTES,
        }
    );
    drop(object);

    let payload_length = usize::try_from(FORMAT_HARD_MAX_SECTION_OR_TABLE_BYTES).unwrap()
        - TABLE_HEADER_BYTES
        - ROW_HEADER_BYTES
        - FIELD_HEADER_BYTES;
    let mut table_bytes = table(
        1,
        &[row(&[field(
            1,
            PortableFieldType::Bytes,
            &vec![0; payload_length],
        )])],
    );
    assert_eq!(
        table_bytes.len() as u64,
        FORMAT_HARD_MAX_SECTION_OR_TABLE_BYTES
    );
    preflight_table_structure_v1(&table_bytes, 1, FormatLimits::V1_HARD).unwrap();
    table_bytes.push(0);
    assert_eq!(
        preflight_table_structure_v1(&table_bytes, 1, FormatLimits::V1_HARD),
        Err(FormatError::LimitExceeded {
            dimension: LimitDimension::SectionOrTableBytes,
            actual: FORMAT_HARD_MAX_SECTION_OR_TABLE_BYTES + 1,
            limit: FORMAT_HARD_MAX_SECTION_OR_TABLE_BYTES,
        })
    );
}

#[test]
fn row_and_field_count_boundaries_accept_limit_and_reject_next_value() {
    let empty = row(&[]);
    let rows = vec![empty; FORMAT_HARD_MAX_ROWS_PER_TABLE as usize];
    let mut rows_table = table(1, &rows);
    preflight_table_structure_v1(&rows_table, 1, FormatLimits::V1_HARD).unwrap();
    rows_table[4..8].copy_from_slice(&(FORMAT_HARD_MAX_ROWS_PER_TABLE + 1).to_le_bytes());
    assert_eq!(
        preflight_table_structure_v1(&rows_table, 1, FormatLimits::V1_HARD),
        Err(FormatError::LimitExceeded {
            dimension: LimitDimension::RowsPerTable,
            actual: u64::from(FORMAT_HARD_MAX_ROWS_PER_TABLE) + 1,
            limit: u64::from(FORMAT_HARD_MAX_ROWS_PER_TABLE),
        })
    );

    let fields = (1..=FORMAT_HARD_MAX_FIELDS_PER_ROW)
        .map(|tag| field(tag as u16, PortableFieldType::U8, &[0]))
        .collect::<Vec<_>>();
    let valid = table(1, &[row(&fields)]);
    preflight_table_structure_v1(&valid, 1, FormatLimits::V1_HARD).unwrap();

    let fields = (1..=FORMAT_HARD_MAX_FIELDS_PER_ROW + 1)
        .map(|tag| field(tag as u16, PortableFieldType::U8, &[0]))
        .collect::<Vec<_>>();
    let invalid = table(1, &[row(&fields)]);
    assert_eq!(
        preflight_table_structure_v1(&invalid, 1, FormatLimits::V1_HARD),
        Err(FormatError::LimitExceeded {
            dimension: LimitDimension::FieldsPerRow,
            actual: u64::from(FORMAT_HARD_MAX_FIELDS_PER_ROW) + 1,
            limit: u64::from(FORMAT_HARD_MAX_FIELDS_PER_ROW),
        })
    );
}

#[test]
fn utf8_field_and_total_boundaries_accept_exact_and_reject_limit_plus_one() {
    let exact_field = vec![b'x'; usize::try_from(FORMAT_HARD_MAX_UTF8_FIELD_BYTES).unwrap()];
    let valid = table(
        1,
        &[row(&[field(1, PortableFieldType::Utf8, &exact_field)])],
    );
    preflight_table_structure_v1(&valid, 1, FormatLimits::V1_HARD).unwrap();
    let mut over_field = exact_field;
    over_field.push(b'x');
    let invalid = table(1, &[row(&[field(1, PortableFieldType::Utf8, &over_field)])]);
    assert_eq!(
        preflight_table_structure_v1(&invalid, 1, FormatLimits::V1_HARD),
        Err(FormatError::LimitExceeded {
            dimension: LimitDimension::Utf8FieldBytes,
            actual: FORMAT_HARD_MAX_UTF8_FIELD_BYTES + 1,
            limit: FORMAT_HARD_MAX_UTF8_FIELD_BYTES,
        })
    );

    let chunk = vec![b'x'; usize::try_from(FORMAT_HARD_MAX_UTF8_FIELD_BYTES).unwrap()];
    let exact_fields = (1..=8)
        .map(|tag| field(tag, PortableFieldType::Utf8, &chunk))
        .collect::<Vec<_>>();
    let exact = table(1, &[row(&exact_fields)]);
    let summary = preflight_table_structure_v1(&exact, 1, FormatLimits::V1_HARD).unwrap();
    assert_eq!(summary.total_utf8_bytes(), FORMAT_HARD_MAX_TOTAL_UTF8_BYTES);
    drop(exact);

    let mut over_fields = exact_fields;
    over_fields.push(field(9, PortableFieldType::Utf8, b"x"));
    let over = table(1, &[row(&over_fields)]);
    assert_eq!(
        preflight_table_structure_v1(&over, 1, FormatLimits::V1_HARD),
        Err(FormatError::LimitExceeded {
            dimension: LimitDimension::TotalUtf8Bytes,
            actual: FORMAT_HARD_MAX_TOTAL_UTF8_BYTES + 1,
            limit: FORMAT_HARD_MAX_TOTAL_UTF8_BYTES,
        })
    );
}

#[test]
fn vector_item_and_total_boundaries_accept_exact_and_reject_next_constructible() {
    let at_item_limit = table(
        1,
        &[row(&[field(
            1,
            PortableFieldType::OrdinalVectorU32,
            &ordinal_vector(FORMAT_HARD_MAX_VECTOR_ITEMS, 0),
        )])],
    );
    preflight_table_structure_v1(&at_item_limit, 1, FormatLimits::V1_HARD).unwrap();
    let over_item_limit = table(
        1,
        &[row(&[field(
            1,
            PortableFieldType::OrdinalVectorU32,
            &ordinal_vector(FORMAT_HARD_MAX_VECTOR_ITEMS + 1, 0),
        )])],
    );
    assert_eq!(
        preflight_table_structure_v1(&over_item_limit, 1, FormatLimits::V1_HARD),
        Err(FormatError::LimitExceeded {
            dimension: LimitDimension::VectorItems,
            actual: u64::from(FORMAT_HARD_MAX_VECTOR_ITEMS) + 1,
            limit: u64::from(FORMAT_HARD_MAX_VECTOR_ITEMS),
        })
    );

    let exact = vector_budget_table(65_504, 0);
    let summary = preflight_table_structure_v1(&exact, 1, FormatLimits::V1_HARD).unwrap();
    assert_eq!(
        summary.total_vector_bytes(),
        FORMAT_HARD_MAX_TOTAL_VECTOR_BYTES
    );
    drop(exact);

    let malformed_plus_one = vector_budget_table(65_504, 1);
    assert_eq!(
        preflight_table_structure_v1(&malformed_plus_one, 1, FormatLimits::V1_HARD),
        Err(FormatError::LimitExceeded {
            dimension: LimitDimension::TotalVectorBytes,
            actual: FORMAT_HARD_MAX_TOTAL_VECTOR_BYTES + 1,
            limit: FORMAT_HARD_MAX_TOTAL_VECTOR_BYTES,
        })
    );
    drop(malformed_plus_one);

    let next_constructible = vector_budget_table(65_505, 0);
    assert_eq!(
        preflight_table_structure_v1(&next_constructible, 1, FormatLimits::V1_HARD),
        Err(FormatError::LimitExceeded {
            dimension: LimitDimension::TotalVectorBytes,
            actual: FORMAT_HARD_MAX_TOTAL_VECTOR_BYTES + 4,
            limit: FORMAT_HARD_MAX_TOTAL_VECTOR_BYTES,
        })
    );
}

#[test]
fn lfsm_source_location_rows_reach_the_frozen_boundary() {
    assert_eq!(
        FORMAT_HARD_MAX_SOURCE_LOCATION_ROWS,
        FORMAT_HARD_MAX_ROWS_PER_TABLE
    );
    let schema = &portable_object_schema(PortableObjectKind::SourceMap).sections[1].tables[2];
    let fields = schema.row.fields[..8]
        .iter()
        .map(|schema_field| {
            let value = if schema_field.tag == 2 {
                vec![0]
            } else {
                vec![0; usize::try_from(schema_field.field_type.fixed_width().unwrap()).unwrap()]
            };
            field(schema_field.tag, schema_field.field_type, &value)
        })
        .collect::<Vec<_>>();
    let text_location = row(&fields);
    let rows = vec![text_location; FORMAT_HARD_MAX_SOURCE_LOCATION_ROWS as usize];
    let mut bytes = table(schema.kind, &rows);
    preflight_table_with_registry_v1(
        &bytes,
        schema,
        FormatLimits::V1_HARD,
        &mut PreflightBudget::default(),
    )
    .unwrap();

    bytes[4..8].copy_from_slice(&(FORMAT_HARD_MAX_SOURCE_LOCATION_ROWS + 1).to_le_bytes());
    assert_eq!(
        preflight_table_with_registry_v1(
            &bytes,
            schema,
            FormatLimits::V1_HARD,
            &mut PreflightBudget::default(),
        ),
        Err(FormatError::LimitExceeded {
            dimension: LimitDimension::RowsPerTable,
            actual: u64::from(FORMAT_HARD_MAX_SOURCE_LOCATION_ROWS) + 1,
            limit: u64::from(FORMAT_HARD_MAX_SOURCE_LOCATION_ROWS),
        })
    );
}
