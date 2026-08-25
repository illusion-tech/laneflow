use super::*;

use std::ops::Range;

use laneflow_format::{
    FormatLimits, RegistryCheckedFieldValue, RegistryCheckedObjectView, RegistryCheckedRowView,
    ValueCheckedObjectView, preflight_object_values,
};
use laneflow_static_contract::{
    EntityKind, OBJECT_PREAMBLE_BYTE_LENGTH, PortableObjectKind,
    SECTION_DIRECTORY_ENTRY_BYTE_LENGTH,
};

fn value_checked(bytes: &[u8], kind: PortableObjectKind) -> ValueCheckedObjectView<'_> {
    preflight_object_values(bytes, kind, FormatLimits::HARD).unwrap()
}

fn registry(bytes: &[u8], kind: PortableObjectKind) -> RegistryCheckedObjectView<'_> {
    value_checked(bytes, kind).registry_view()
}

fn row(
    bytes: &[u8],
    kind: PortableObjectKind,
    section: u32,
    table: u32,
    row: u32,
) -> RegistryCheckedRowView<'_> {
    registry(bytes, kind)
        .section(section)
        .and_then(|section| section.table(table))
        .and_then(|table| table.row(row))
        .unwrap()
}

fn field_value_range(
    bytes: &[u8],
    kind: PortableObjectKind,
    section: u32,
    table: u32,
    row: u32,
    tag: u16,
) -> Range<usize> {
    let value = self::row(bytes, kind, section, table, row)
        .field_by_tag(tag)
        .unwrap()
        .value_bytes();
    let start = value.as_ptr() as usize - bytes.as_ptr() as usize;
    start..start + value.len()
}

fn copy_field_value(
    bytes: &mut [u8],
    kind: PortableObjectKind,
    from: (u32, u32, u32, u16),
    to: (u32, u32, u32, u16),
) {
    let from = field_value_range(bytes, kind, from.0, from.1, from.2, from.3);
    let to = field_value_range(bytes, kind, to.0, to.1, to.2, to.3);
    assert_eq!(from.len(), to.len());
    let value = bytes[from].to_vec();
    bytes[to].copy_from_slice(&value);
}

fn remove_field(
    bytes: &mut Vec<u8>,
    kind: PortableObjectKind,
    section_ordinal: u32,
    table_ordinal: u32,
    row_ordinal: u32,
    tag: u16,
) {
    const FIELD_HEADER_BYTES: usize = 12;
    let view = registry(bytes, kind);
    let section = view.section(section_ordinal).unwrap();
    let table = section.table(table_ordinal).unwrap();
    let row = table.row(row_ordinal).unwrap();
    let value = row.field_by_tag(tag).unwrap().value_bytes();
    let object_start = bytes.as_ptr() as usize;
    let section_start = section.bytes().as_ptr() as usize - object_start;
    let table_start = table.bytes().as_ptr() as usize - object_start;
    let row_start = row.bytes().as_ptr() as usize - object_start;
    let value_start = value.as_ptr() as usize - object_start;
    let field_start = value_start - FIELD_HEADER_BYTES;
    let field_end = value_start + value.len();
    let removed = u64::try_from(field_end - field_start).unwrap();

    let row_length = u64::from_le_bytes(bytes[row_start..row_start + 8].try_into().unwrap());
    bytes[row_start..row_start + 8].copy_from_slice(&(row_length - removed).to_le_bytes());
    let field_count = u32::from_le_bytes(bytes[row_start + 8..row_start + 12].try_into().unwrap());
    bytes[row_start + 8..row_start + 12].copy_from_slice(&(field_count - 1).to_le_bytes());
    let rows_length =
        u64::from_le_bytes(bytes[table_start + 8..table_start + 16].try_into().unwrap());
    bytes[table_start + 8..table_start + 16]
        .copy_from_slice(&(rows_length - removed).to_le_bytes());

    let directory_entry = usize::from(OBJECT_PREAMBLE_BYTE_LENGTH)
        + usize::try_from(section_ordinal).unwrap()
            * usize::try_from(SECTION_DIRECTORY_ENTRY_BYTE_LENGTH).unwrap();
    let section_length = u64::from_le_bytes(
        bytes[directory_entry + 16..directory_entry + 24]
            .try_into()
            .unwrap(),
    );
    assert_eq!(
        section_start,
        usize::try_from(u64::from_le_bytes(
            bytes[directory_entry + 8..directory_entry + 16]
                .try_into()
                .unwrap()
        ))
        .unwrap()
    );
    bytes[directory_entry + 16..directory_entry + 24]
        .copy_from_slice(&(section_length - removed).to_le_bytes());
    for following in section_ordinal + 1..kind.section_count() {
        let entry = usize::from(OBJECT_PREAMBLE_BYTE_LENGTH)
            + usize::try_from(following).unwrap()
                * usize::try_from(SECTION_DIRECTORY_ENTRY_BYTE_LENGTH).unwrap();
        let offset = u64::from_le_bytes(bytes[entry + 8..entry + 16].try_into().unwrap()) - removed;
        bytes[entry + 8..entry + 16].copy_from_slice(&offset.to_le_bytes());
    }
    let object_length = u64::from_le_bytes(bytes[24..32].try_into().unwrap());
    bytes[24..32].copy_from_slice(&(object_length - removed).to_le_bytes());
    bytes.drain(field_start..field_end);
}

fn field_u8(row: RegistryCheckedRowView<'_>, tag: u16) -> u8 {
    match row.field_by_tag(tag).unwrap().value().unwrap() {
        RegistryCheckedFieldValue::U8(value) => value,
        value => panic!("expected u8 field {tag}, got {value:?}"),
    }
}

fn field_u16(row: RegistryCheckedRowView<'_>, tag: u16) -> u16 {
    match row.field_by_tag(tag).unwrap().value().unwrap() {
        RegistryCheckedFieldValue::U16(value) => value,
        value => panic!("expected u16 field {tag}, got {value:?}"),
    }
}

fn field_u32(row: RegistryCheckedRowView<'_>, tag: u16) -> u32 {
    match row.field_by_tag(tag).unwrap().value().unwrap() {
        RegistryCheckedFieldValue::U32(value) => value,
        value => panic!("expected u32 field {tag}, got {value:?}"),
    }
}

fn field_u64(row: RegistryCheckedRowView<'_>, tag: u16) -> u64 {
    match row.field_by_tag(tag).unwrap().value().unwrap() {
        RegistryCheckedFieldValue::U64(value) => value,
        value => panic!("expected u64 field {tag}, got {value:?}"),
    }
}

fn field_stable_id(row: RegistryCheckedRowView<'_>, tag: u16) -> [u8; 16] {
    match row.field_by_tag(tag).unwrap().value().unwrap() {
        RegistryCheckedFieldValue::StableId128(value) => value.into_bytes(),
        value => panic!("expected StableId128 field {tag}, got {value:?}"),
    }
}

fn field_sha256(row: RegistryCheckedRowView<'_>, tag: u16) -> [u8; 32] {
    match row.field_by_tag(tag).unwrap().value().unwrap() {
        RegistryCheckedFieldValue::Sha256(value) => value.into_bytes(),
        value => panic!("expected Sha256 field {tag}, got {value:?}"),
    }
}

fn field_utf8(row: RegistryCheckedRowView<'_>, tag: u16) -> &str {
    match row.field_by_tag(tag).unwrap().value().unwrap() {
        RegistryCheckedFieldValue::Utf8(value) => value,
        value => panic!("expected Utf8 field {tag}, got {value:?}"),
    }
}

fn field_ordinals(row: RegistryCheckedRowView<'_>, tag: u16) -> Vec<u32> {
    match row.field_by_tag(tag).unwrap().value().unwrap() {
        RegistryCheckedFieldValue::OrdinalVectorU32(values) => (0..values.len())
            .map(|index| values.get(index).unwrap())
            .collect(),
        value => panic!("expected OrdinalVectorU32 field {tag}, got {value:?}"),
    }
}

fn entity_table_ordinal(kind: EntityKind) -> u32 {
    EntityKind::ALL
        .into_iter()
        .position(|candidate| candidate == kind)
        .and_then(|ordinal| u32::try_from(ordinal).ok())
        .unwrap()
}

mod art;
mod diff;
mod map;
