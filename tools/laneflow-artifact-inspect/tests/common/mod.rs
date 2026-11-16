#![allow(dead_code)]

//! 集成测试共享工厂：经正式编码路径构造最小合法对象。

use laneflow_format::{
    FieldWriteInput, FieldWriteValue, FormatLimits, ObjectWriteInput, RowWriteInput,
    SectionWriteInput, TableWriteInput, encode_object, measure_object,
};
use laneflow_static_contract::{
    PortableFieldPresence, PortableObjectKind, PortableRowCardinality, PortableRowSchema,
    portable_object_schema,
};

/// 按登记 schema 构造一行：Uniform 取全部 Required 字段默认值；
/// DiscriminatedU8 取第一个变体（含判别值、required 与 at_least_one 位）。
fn minimal_row(schema: &PortableRowSchema) -> RowWriteInput<'static> {
    let mut fields: Vec<FieldWriteInput<'static>> = Vec::new();
    if let Some((tag, variants)) = schema.shape.discriminant()
        && let Some(variant) = variants.first()
    {
        let bits = variant.required_fields | variant.at_least_one_field;
        for field in schema.fields {
            let mask = laneflow_static_contract::portable_field_mask(field.tag);
            if field.tag == tag {
                fields.push(FieldWriteInput {
                    tag: field.tag,
                    value: FieldWriteValue::U8(variant.discriminant),
                });
            } else if bits & mask != 0
                && let Some(value) = default_value(field)
            {
                fields.push(FieldWriteInput {
                    tag: field.tag,
                    value,
                });
            }
        }
        return RowWriteInput {
            fields: leak_fields(fields),
        };
    }
    for field in schema.fields {
        if field.presence == PortableFieldPresence::Required
            && let Some(value) = default_value(field)
        {
            fields.push(FieldWriteInput {
                tag: field.tag,
                value,
            });
        }
    }
    RowWriteInput {
        fields: leak_fields(fields),
    }
}

// 集成测试进程内一次性构造；静态默认值本身即为 'static 数据，
// 这里仅把 Vec 固化为运行期不变的借用切片。
fn leak_fields(fields: Vec<FieldWriteInput<'static>>) -> &'static [FieldWriteInput<'static>] {
    Box::leak(fields.into_boxed_slice())
}

fn default_value(
    field: &laneflow_static_contract::PortableFieldSchema,
) -> Option<FieldWriteValue<'static>> {
    use laneflow_static_contract::PortableFieldType as T;
    Some(match field.field_type {
        T::U8 => FieldWriteValue::U8(0),
        T::U16 => FieldWriteValue::U16(0),
        T::U32 => FieldWriteValue::U32(0),
        T::U64 => FieldWriteValue::U64(0),
        T::I32 => FieldWriteValue::I32(0),
        T::F32 => FieldWriteValue::F32(1.0),
        T::F64 => FieldWriteValue::F64(1.0),
        T::StableId128 => FieldWriteValue::StableId128([0; 16]),
        T::Sha256 => FieldWriteValue::Sha256([0; 32]),
        T::Utf8 => FieldWriteValue::Utf8(""),
        T::Bytes => return None, // Bytes 默认值语义不定，交给调用方
        T::OrdinalVectorU32 => FieldWriteValue::OrdinalVectorU32(&[]),
        T::RecordVector => return None, // 嵌套行由调用方构造
    })
}

/// 构造某类对象的最小合法字节：required singleton 表恰一行，其余表零行。
pub fn minimal_object(kind: PortableObjectKind) -> Vec<u8> {
    let schema = portable_object_schema(kind);
    let mut sections: Vec<SectionWriteInput<'static>> = Vec::new();
    for section in schema.sections {
        let mut tables: Vec<TableWriteInput<'static>> = Vec::new();
        for table in section.tables {
            // LFSD PolicyLocalChange 的 Bytes 成员默认值不合法，最小对象取零行。
            let rows: &[RowWriteInput<'static>] = match table.cardinality {
                PortableRowCardinality::ExactlyOne => {
                    if table
                        .row
                        .fields
                        .iter()
                        .any(|field| default_value(field).is_none())
                    {
                        &[]
                    } else {
                        leak_rows(vec![minimal_row(table.row)])
                    }
                }
                _ => &[],
            };
            tables.push(TableWriteInput {
                kind: table.kind,
                rows,
            });
        }
        sections.push(SectionWriteInput {
            kind: section.kind,
            tables: Box::leak(tables.into_boxed_slice()),
        });
    }
    let input = ObjectWriteInput {
        kind,
        sections: Box::leak(sections.into_boxed_slice()),
    };
    let length = measure_object(input, FormatLimits::HARD).expect("最小对象应可通过写入预检");
    let mut bytes = vec![0_u8; usize::try_from(length).expect("长度")];
    encode_object(input, FormatLimits::HARD, &mut bytes).expect("编码最小对象");
    bytes
}

fn leak_rows(rows: Vec<RowWriteInput<'static>>) -> &'static [RowWriteInput<'static>] {
    Box::leak(rows.into_boxed_slice())
}

/// 构造一行 SourceLocation（variant 0，tags 1..=8）。
pub fn source_location_row(
    ordinal: u32,
    start_line: u32,
    start_column: u32,
    end_line: u32,
    end_column: u32,
) -> RowWriteInput<'static> {
    let fields: Vec<FieldWriteInput<'static>> = vec![
        FieldWriteInput {
            tag: 1,
            value: FieldWriteValue::U32(ordinal),
        },
        FieldWriteInput {
            tag: 2,
            value: FieldWriteValue::U8(0),
        },
        FieldWriteInput {
            tag: 3,
            value: FieldWriteValue::U32(0),
        },
        FieldWriteInput {
            tag: 4,
            value: FieldWriteValue::U32(0),
        },
        FieldWriteInput {
            tag: 5,
            value: FieldWriteValue::U32(start_line),
        },
        FieldWriteInput {
            tag: 6,
            value: FieldWriteValue::U32(start_column),
        },
        FieldWriteInput {
            tag: 7,
            value: FieldWriteValue::U32(end_line),
        },
        FieldWriteInput {
            tag: 8,
            value: FieldWriteValue::U32(end_column),
        },
    ];
    RowWriteInput {
        fields: leak_fields(fields),
    }
}

/// 构造带 SourceLocation 行的 LFSM（其余表保持最小对象形状）。
pub fn lfsm_with_source_locations(start_lines: &[u32]) -> Vec<u8> {
    let schema = portable_object_schema(PortableObjectKind::SourceMap);
    let mut sections: Vec<SectionWriteInput<'static>> = Vec::new();
    for section in schema.sections {
        let mut tables: Vec<TableWriteInput<'static>> = Vec::new();
        for table in section.tables {
            let rows: Vec<RowWriteInput<'static>> = if table.kind == 3 {
                start_lines
                    .iter()
                    .enumerate()
                    .map(|(index, start)| {
                        source_location_row(
                            u32::try_from(index).expect("序号"),
                            *start,
                            1,
                            *start + 1,
                            2,
                        )
                    })
                    .collect()
            } else {
                match table.cardinality {
                    PortableRowCardinality::ExactlyOne => {
                        if table
                            .row
                            .fields
                            .iter()
                            .any(|field| default_value(field).is_none())
                        {
                            Vec::new()
                        } else {
                            vec![minimal_row(table.row)]
                        }
                    }
                    _ => Vec::new(),
                }
            };
            tables.push(TableWriteInput {
                kind: table.kind,
                rows: leak_rows(rows),
            });
        }
        sections.push(SectionWriteInput {
            kind: section.kind,
            tables: Box::leak(tables.into_boxed_slice()),
        });
    }
    let input = ObjectWriteInput {
        kind: PortableObjectKind::SourceMap,
        sections: Box::leak(sections.into_boxed_slice()),
    };
    let length = measure_object(input, FormatLimits::HARD).expect("LFSM 应通过写入预检");
    let mut bytes = vec![0_u8; usize::try_from(length).expect("长度")];
    encode_object(input, FormatLimits::HARD, &mut bytes).expect("编码 LFSM");
    bytes
}

/// 构造 kind 0（evidence）成员值的完整 RowV1 字节。
///
/// 测试夹具按线格式文档手工装配（与 laneflow-format 自身测试的装配方式一致）：
/// 16 字节行头 + 单个 Utf8 字段（tag 3，12 字节字段头 + 值）。
pub fn evidence_member_bytes(locator: &str) -> Vec<u8> {
    let value = locator.as_bytes();
    let mut bytes = Vec::with_capacity(28 + value.len());
    bytes.extend_from_slice(&(16 + 12 + value.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes.extend_from_slice(&3_u16.to_le_bytes());
    bytes.push(9);
    bytes.push(0);
    bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
    bytes.extend_from_slice(value);
    bytes
}

/// 构造带一条 PolicyLocalChange 行（kind 0 evidence 成员，before/after 各一份）的 LFSD。
pub fn lfsd_with_evidence_change(key: &str, before_locator: &str, after_locator: &str) -> Vec<u8> {
    let schema = portable_object_schema(PortableObjectKind::SemanticDiff);
    let mut sections: Vec<SectionWriteInput<'static>> = Vec::new();
    for section in schema.sections {
        let mut tables: Vec<TableWriteInput<'static>> = Vec::new();
        for table in section.tables {
            let rows: Vec<RowWriteInput<'static>> = if table.kind == 1 && section.kind == 7 {
                let fields: Vec<FieldWriteInput<'static>> = vec![
                    FieldWriteInput {
                        tag: 1,
                        value: FieldWriteValue::U8(2),
                    },
                    FieldWriteInput {
                        tag: 2,
                        value: FieldWriteValue::StableId128([1; 16]),
                    },
                    FieldWriteInput {
                        tag: 3,
                        value: FieldWriteValue::U8(0),
                    },
                    FieldWriteInput {
                        tag: 4,
                        value: FieldWriteValue::Utf8(Box::leak(key.to_string().into_boxed_str())),
                    },
                    FieldWriteInput {
                        tag: 5,
                        value: FieldWriteValue::Bytes(Box::leak(
                            evidence_member_bytes(before_locator).into_boxed_slice(),
                        )),
                    },
                    FieldWriteInput {
                        tag: 6,
                        value: FieldWriteValue::Bytes(Box::leak(
                            evidence_member_bytes(after_locator).into_boxed_slice(),
                        )),
                    },
                ];
                vec![RowWriteInput {
                    fields: leak_fields(fields),
                }]
            } else {
                match table.cardinality {
                    PortableRowCardinality::ExactlyOne => {
                        if table
                            .row
                            .fields
                            .iter()
                            .any(|field| default_value(field).is_none())
                        {
                            Vec::new()
                        } else {
                            vec![minimal_row(table.row)]
                        }
                    }
                    _ => Vec::new(),
                }
            };
            tables.push(TableWriteInput {
                kind: table.kind,
                rows: leak_rows(rows),
            });
        }
        sections.push(SectionWriteInput {
            kind: section.kind,
            tables: Box::leak(tables.into_boxed_slice()),
        });
    }
    let input = ObjectWriteInput {
        kind: PortableObjectKind::SemanticDiff,
        sections: Box::leak(sections.into_boxed_slice()),
    };
    let length = measure_object(input, FormatLimits::HARD).expect("LFSD 应通过写入预检");
    let mut bytes = vec![0_u8; usize::try_from(length).expect("长度")];
    encode_object(input, FormatLimits::HARD, &mut bytes).expect("编码 LFSD");
    bytes
}

/// 构造带一行 CanonicalIdentity（identityFields 嵌套 Bytes 值）的 LFCA。
pub fn lfca_with_identity_field_values(values: &[&[u8]]) -> Vec<u8> {
    let schema = portable_object_schema(PortableObjectKind::CanonicalArtifact);
    let mut sections: Vec<SectionWriteInput<'static>> = Vec::new();
    for section in schema.sections {
        let mut tables: Vec<TableWriteInput<'static>> = Vec::new();
        for table in section.tables {
            let rows: Vec<RowWriteInput<'static>> = if table.kind == 1 && section.kind == 2 {
                let nested: Vec<RowWriteInput<'static>> = values
                    .iter()
                    .map(|value| RowWriteInput {
                        fields: leak_fields(vec![
                            FieldWriteInput {
                                tag: 1,
                                value: FieldWriteValue::U16(7),
                            },
                            FieldWriteInput {
                                tag: 2,
                                value: FieldWriteValue::Bytes(Box::leak(
                                    value.to_vec().into_boxed_slice(),
                                )),
                            },
                        ]),
                    })
                    .collect();
                let row_fields: Vec<FieldWriteInput<'static>> = vec![
                    FieldWriteInput {
                        tag: 1,
                        value: FieldWriteValue::U16(0),
                    },
                    FieldWriteInput {
                        tag: 2,
                        value: FieldWriteValue::U32(0),
                    },
                    FieldWriteInput {
                        tag: 3,
                        value: FieldWriteValue::StableId128([2; 16]),
                    },
                    FieldWriteInput {
                        tag: 4,
                        value: FieldWriteValue::RecordVector(leak_rows(nested)),
                    },
                ];
                vec![RowWriteInput {
                    fields: leak_fields(row_fields),
                }]
            } else {
                match table.cardinality {
                    PortableRowCardinality::ExactlyOne => {
                        if table
                            .row
                            .fields
                            .iter()
                            .any(|field| default_value(field).is_none())
                        {
                            Vec::new()
                        } else {
                            vec![minimal_row(table.row)]
                        }
                    }
                    _ => Vec::new(),
                }
            };
            tables.push(TableWriteInput {
                kind: table.kind,
                rows: leak_rows(rows),
            });
        }
        sections.push(SectionWriteInput {
            kind: section.kind,
            tables: Box::leak(tables.into_boxed_slice()),
        });
    }
    let input = ObjectWriteInput {
        kind: PortableObjectKind::CanonicalArtifact,
        sections: Box::leak(sections.into_boxed_slice()),
    };
    let length = measure_object(input, FormatLimits::HARD).expect("LFCA 应通过写入预检");
    let mut bytes = vec![0_u8; usize::try_from(length).expect("长度")];
    encode_object(input, FormatLimits::HARD, &mut bytes).expect("编码 LFCA");
    bytes
}
