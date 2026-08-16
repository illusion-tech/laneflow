use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum OwnedValue {
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
pub(super) struct OwnedField {
    pub(super) tag: u16,
    pub(super) value: OwnedValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct OwnedRow {
    pub(super) fields: Box<[OwnedField]>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct OwnedTable {
    pub(super) kind: u16,
    pub(super) rows: Box<[OwnedRow]>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct OwnedSection {
    pub(super) kind: u16,
    pub(super) tables: Box<[OwnedTable]>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct OwnedObject {
    pub(super) kind: PortableObjectKind,
    pub(super) sections: Box<[OwnedSection]>,
}

pub(super) fn field(tag: u16, value: OwnedValue) -> OwnedField {
    OwnedField { tag, value }
}

pub(super) fn row(fields: impl IntoIterator<Item = OwnedField>) -> OwnedRow {
    OwnedRow {
        fields: fields.into_iter().collect(),
    }
}

pub(super) fn table(kind: u16, rows: impl IntoIterator<Item = OwnedRow>) -> OwnedTable {
    OwnedTable {
        kind,
        rows: rows.into_iter().collect(),
    }
}

pub(super) fn section(kind: u16, tables: impl IntoIterator<Item = OwnedTable>) -> OwnedSection {
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
pub(super) fn encode_owned_object(
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
