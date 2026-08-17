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

#[derive(Clone, Copy, Debug)]
struct ArenaSpan {
    start: usize,
    end: usize,
}

impl ArenaSpan {
    fn from_start(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    fn slice<T>(self, values: &[T]) -> &[T] {
        &values[self.start..self.end]
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ArenaCapacities {
    nested_fields: usize,
    nested_rows: usize,
    record_vectors: usize,
    top_fields: usize,
    top_rows: usize,
    tables: usize,
    sections: usize,
}

impl ArenaCapacities {
    fn for_object(object: &OwnedObject) -> Result<Self, PortableEmissionError> {
        fn add(total: &mut usize, value: usize) -> Result<(), PortableEmissionError> {
            *total = total
                .checked_add(value)
                .ok_or(PortableEmissionError::ArithmeticOverflow)?;
            Ok(())
        }

        let mut capacities = Self {
            sections: object.sections.len(),
            ..Self::default()
        };
        for section in &object.sections {
            add(&mut capacities.tables, section.tables.len())?;
            for table in &section.tables {
                add(&mut capacities.top_rows, table.rows.len())?;
                for row in &table.rows {
                    add(&mut capacities.top_fields, row.fields.len())?;
                    for field in &row.fields {
                        if let OwnedValue::RecordVector(rows) = &field.value {
                            add(&mut capacities.record_vectors, 1)?;
                            add(&mut capacities.nested_rows, rows.len())?;
                            for nested_row in rows {
                                if nested_row.fields.iter().any(|field| {
                                    matches!(&field.value, OwnedValue::RecordVector(_))
                                }) {
                                    return Err(PortableEmissionError::InternalBindingMismatch);
                                }
                                add(&mut capacities.nested_fields, nested_row.fields.len())?;
                            }
                        }
                    }
                }
            }
        }
        Ok(capacities)
    }
}

/// 把拥有型 compiler projection 临时降低为 `laneflow-format` 的零分配借用 writer 输入。
///
/// v1 只允许一层 RecordVector，因此各借用层按 nested fields -> nested rows -> top fields
/// -> top rows -> tables -> sections 的顺序建立。每一层使用一个连续 arena 和轻量 span，
/// 不为每行/每表复制一棵 Box 树，也没有自引用拥有结构或泄漏分配。
pub(super) fn encode_owned_object(
    object: &OwnedObject,
    limits: FormatLimits,
    already_staged_bytes: u64,
) -> Result<Box<[u8]>, PortableEmissionError> {
    let capacities = ArenaCapacities::for_object(object)?;
    let mut nested_fields = Vec::<FieldWriteInputV1<'_>>::with_capacity(capacities.nested_fields);
    let mut nested_field_spans = Vec::<ArenaSpan>::with_capacity(capacities.nested_rows);
    let mut record_row_spans = Vec::<ArenaSpan>::with_capacity(capacities.record_vectors);
    for section in &object.sections {
        for table in &section.tables {
            for row in &table.rows {
                for field in &row.fields {
                    if let OwnedValue::RecordVector(rows) = &field.value {
                        let record_start = nested_field_spans.len();
                        for nested_row in rows {
                            let field_start = nested_fields.len();
                            nested_fields.extend(nested_row.fields.iter().map(|field| {
                                FieldWriteInputV1 {
                                    tag: field.tag,
                                    value: borrow_primitive_value(&field.value),
                                }
                            }));
                            nested_field_spans
                                .push(ArenaSpan::from_start(field_start, nested_fields.len()));
                        }
                        record_row_spans.push(ArenaSpan::from_start(
                            record_start,
                            nested_field_spans.len(),
                        ));
                    }
                }
            }
        }
    }

    let mut nested_rows = Vec::with_capacity(capacities.nested_rows);
    nested_rows.extend(nested_field_spans.iter().map(|span| RowWriteInputV1 {
        fields: span.slice(&nested_fields),
    }));

    let mut record_rows = record_row_spans.iter();
    let mut top_fields = Vec::<FieldWriteInputV1<'_>>::with_capacity(capacities.top_fields);
    let mut top_field_spans = Vec::<ArenaSpan>::with_capacity(capacities.top_rows);
    for section in &object.sections {
        for table in &section.tables {
            for row in &table.rows {
                let field_start = top_fields.len();
                for field in &row.fields {
                    let value = match &field.value {
                        OwnedValue::RecordVector(_) => FieldWriteValueV1::RecordVector(
                            record_rows
                                .next()
                                .ok_or(PortableEmissionError::InternalBindingMismatch)?
                                .slice(&nested_rows),
                        ),
                        value => borrow_primitive_value(value),
                    };
                    top_fields.push(FieldWriteInputV1 {
                        tag: field.tag,
                        value,
                    });
                }
                top_field_spans.push(ArenaSpan::from_start(field_start, top_fields.len()));
            }
        }
    }
    debug_assert!(record_rows.next().is_none());

    let mut top_rows = Vec::with_capacity(capacities.top_rows);
    top_rows.extend(top_field_spans.iter().map(|span| RowWriteInputV1 {
        fields: span.slice(&top_fields),
    }));

    let mut top_row_index = 0_usize;
    let mut table_row_spans = Vec::<ArenaSpan>::with_capacity(capacities.tables);
    for section in &object.sections {
        for table in &section.tables {
            let start = top_row_index;
            top_row_index += table.rows.len();
            table_row_spans.push(ArenaSpan::from_start(start, top_row_index));
        }
    }
    debug_assert_eq!(top_row_index, top_rows.len());

    let mut table_rows = table_row_spans.iter();
    let mut tables = Vec::<TableWriteInputV1<'_>>::with_capacity(capacities.tables);
    let mut section_table_spans = Vec::<ArenaSpan>::with_capacity(capacities.sections);
    for section in &object.sections {
        let table_start = tables.len();
        for table in &section.tables {
            tables.push(TableWriteInputV1 {
                kind: table.kind,
                rows: table_rows
                    .next()
                    .ok_or(PortableEmissionError::InternalBindingMismatch)?
                    .slice(&top_rows),
            });
        }
        section_table_spans.push(ArenaSpan::from_start(table_start, tables.len()));
    }
    debug_assert!(table_rows.next().is_none());

    let mut sections = Vec::with_capacity(capacities.sections);
    sections.extend(
        object
            .sections
            .iter()
            .zip(&section_table_spans)
            .map(|(section, span)| SectionWriteInputV1 {
                kind: section.kind,
                tables: span.slice(&tables),
            }),
    );
    let input = ObjectWriteInputV1 {
        kind: object.kind,
        sections: &sections,
    };
    let prepared = prepare_object_v1(input, limits)?;
    let length = prepared.byte_len();
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
    encode_prepared_object_v1(prepared, &mut bytes)?;
    Ok(bytes.into_boxed_slice())
}
