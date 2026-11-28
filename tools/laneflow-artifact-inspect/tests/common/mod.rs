#![allow(dead_code)]

//! 集成测试共享工厂：经正式编码路径构造同时通过 registry 结构与直接值域检查的
//! 最小合法对象。

use laneflow_format::{
    FieldWriteInput, FieldWriteValue, FormatLimits, ObjectWriteInput, RowWriteInput,
    SectionWriteInput, TableWriteInput, encode_object, measure_object, preflight_object_values,
};
use laneflow_static_contract::{
    CANONICAL_ARTIFACT_FORMAT_VERSION, CONSTRAINT_CONTRACT_VERSION, EntityKind,
    IDENTITY_REGISTRY_REVISION, NETWORK_REVISION_DERIVATION_VERSION, PortableFieldPresence,
    PortableObjectKind, PortableRowCardinality, PortableRowSchema, SOURCE_MAP_FORMAT_VERSION,
    STATIC_EXECUTION_CONTRACT_VERSION, portable_object_schema,
};

/// LFCA 编译选项行登记的选项摘要；与 `laneflow-format` 直接值域检查的登记值一致，
/// 该值变化时 [`encode`] 的值域断言会使本工厂失败。
const PORTABLE_COMPILE_OPTIONS_DIGEST_V1: [u8; 32] = [
    0x32, 0x26, 0x82, 0xf4, 0x55, 0xd0, 0x6b, 0x36, 0xe9, 0xe3, 0x71, 0x9f, 0x34, 0x1d, 0xb3, 0x8f,
    0x3e, 0xcd, 0xa6, 0x1d, 0x52, 0xc5, 0x3d, 0x9d, 0x6f, 0xe3, 0xdc, 0xa5, 0x40, 0xee, 0xf4, 0x45,
];

/// LFCP 两条制品绑定共用的非零摘要；对象键按它派生。
const BOUND_DIGEST: [u8; 32] = [0x11; 32];

/// 按登记 schema 构造一行：Uniform 取全部 Required 字段默认值；
/// DiscriminatedU8 取第一个变体（含判别值、required 与 at_least_one 位）。
fn minimal_row(
    kind: PortableObjectKind,
    section: u16,
    table: u16,
    schema: &PortableRowSchema,
) -> RowWriteInput<'static> {
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
                && let Some(value) = default_value(kind, section, table, field)
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
            && let Some(value) = default_value(kind, section, table, field)
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

/// 字段的最小合法值：版本、修订派生、长度、摘要与对象键按直接值域检查的
/// 登记值填写，其余取零或空。Bytes 与 RecordVector 的语义由调用方决定。
fn default_value(
    kind: PortableObjectKind,
    section: u16,
    table: u16,
    field: &laneflow_static_contract::PortableFieldSchema,
) -> Option<FieldWriteValue<'static>> {
    use PortableObjectKind as K;
    use laneflow_static_contract::PortableFieldType as T;
    let key = (kind, section, table, field.tag);
    Some(match field.field_type {
        T::U8 => FieldWriteValue::U8(0),
        T::U16 => FieldWriteValue::U16(match key {
            (K::CanonicalArtifact, 1, 1, 1)
            | (K::SourceMap, 1, 1, 3)
            | (K::CanonicalPublicationDescriptor, 1, 1, 1) => CANONICAL_ARTIFACT_FORMAT_VERSION,
            (K::CanonicalPublicationDescriptor, 2, 1, 1) => SOURCE_MAP_FORMAT_VERSION,
            (K::CanonicalArtifact, 1, 1, 5) | (K::CanonicalArtifact, 6, 1, 2) => {
                CONSTRAINT_CONTRACT_VERSION
            }
            (K::CanonicalArtifact, 1, 1, 6) | (K::CanonicalArtifact, 6, 1, 1) => {
                STATIC_EXECUTION_CONTRACT_VERSION
            }
            (K::CanonicalArtifact, 1, 1, 3) => IDENTITY_REGISTRY_REVISION,
            (K::CanonicalArtifact, 7, 1, 5) => 2,
            (K::CanonicalArtifact, 1, 1, 4)
            | (K::SourceMap, 1, 1, 1)
            | (K::SemanticDiff, 1, 1, 6)
            | (K::CanonicalPublicationDescriptor, 1, 1, 2) => NETWORK_REVISION_DERIVATION_VERSION,
            (K::CanonicalArtifact, 1, 1, 2)
            | (K::CanonicalArtifact, 7, 1, 2)
            | (K::SourceMap, 1, 1, 7)
            | (K::CanonicalPublicationDescriptor, 2, 1, 5) => 1,
            _ => 0,
        }),
        T::U32 => FieldWriteValue::U32(0),
        T::U64 => FieldWriteValue::U64(match key {
            (K::SourceMap, 1, 1, 5)
            | (K::SemanticDiff, 1, 1, 9)
            | (K::CanonicalPublicationDescriptor, 1, 1, 5)
            | (K::CanonicalPublicationDescriptor, 2, 1, 3) => 1,
            _ => 0,
        }),
        T::I32 => FieldWriteValue::I32(0),
        T::F32 => FieldWriteValue::F32(1.0),
        T::F64 => FieldWriteValue::F64(1.0),
        T::StableId128 => FieldWriteValue::StableId128([0; 16]),
        T::Sha256 => FieldWriteValue::Sha256(match key {
            (K::CanonicalArtifact, 7, _, 4) => PORTABLE_COMPILE_OPTIONS_DIGEST_V1,
            (K::SemanticDiff, 1, _, 7 | 8)
            | (K::CanonicalPublicationDescriptor, 1, _, 4)
            | (K::CanonicalPublicationDescriptor, 2, _, 2) => BOUND_DIGEST,
            _ => [0; 32],
        }),
        T::Utf8 => FieldWriteValue::Utf8(match key {
            (K::CanonicalPublicationDescriptor, 3, 1, 3 | 4) => bound_object_key(),
            _ => "build-v1",
        }),
        T::Bytes => return None,
        T::OrdinalVectorU32 => FieldWriteValue::OrdinalVectorU32(&[]),
        T::RecordVector => return None,
    })
}

/// [`BOUND_DIGEST`] 的 `sha256/<小写十六进制>` 对象键。
fn bound_object_key() -> &'static str {
    let mut key = String::from("sha256/");
    for byte in BOUND_DIGEST {
        key.push_str(&format!("{byte:02x}"));
    }
    Box::leak(key.into_boxed_str())
}

/// 表无法由默认值构造时（含 Bytes 或 RecordVector 必填字段）取零行。
fn has_unbuildable_field(row: &PortableRowSchema) -> bool {
    row.fields.iter().any(|field| {
        matches!(
            field.field_type,
            laneflow_static_contract::PortableFieldType::Bytes
                | laneflow_static_contract::PortableFieldType::RecordVector
        )
    })
}

/// 单例表恰一行（可构造时），其余表零行。
fn default_rows(
    kind: PortableObjectKind,
    section: u16,
    table: &laneflow_static_contract::PortableTableSchema,
) -> Vec<RowWriteInput<'static>> {
    match table.cardinality {
        PortableRowCardinality::ExactlyOne if !has_unbuildable_field(table.row) => {
            vec![minimal_row(kind, section, table.kind, table.row)]
        }
        _ => Vec::new(),
    }
}

/// 按节、表逐一取行后经正式写入路径编码，并断言结果通过直接值域检查。
fn encode(
    kind: PortableObjectKind,
    rows_for: impl FnMut(
        u16,
        &laneflow_static_contract::PortableTableSchema,
    ) -> Vec<RowWriteInput<'static>>,
) -> Vec<u8> {
    let bytes = encode_structure(kind, rows_for);
    if let Err(error) = preflight_object_values(&bytes, kind, FormatLimits::HARD) {
        panic!("测试工厂构造的 {kind:?} 未通过直接值域检查：{error:?}");
    }
    bytes
}

/// `checked` 为假时只做结构编码，仅供构造值域负例。
fn build(
    kind: PortableObjectKind,
    checked: bool,
    rows_for: impl FnMut(
        u16,
        &laneflow_static_contract::PortableTableSchema,
    ) -> Vec<RowWriteInput<'static>>,
) -> Vec<u8> {
    if checked {
        encode(kind, rows_for)
    } else {
        encode_structure(kind, rows_for)
    }
}

/// 只经写入路径的结构检查编码，不断言值域；仅供构造值域负例。
fn encode_structure(
    kind: PortableObjectKind,
    mut rows_for: impl FnMut(
        u16,
        &laneflow_static_contract::PortableTableSchema,
    ) -> Vec<RowWriteInput<'static>>,
) -> Vec<u8> {
    let schema = portable_object_schema(kind);
    let mut sections: Vec<SectionWriteInput<'static>> = Vec::new();
    for section in schema.sections {
        let mut tables: Vec<TableWriteInput<'static>> = Vec::new();
        for table in section.tables {
            tables.push(TableWriteInput {
                kind: table.kind,
                rows: leak_rows(rows_for(section.kind, table)),
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
    let length = measure_object(input, FormatLimits::HARD).expect("对象应可通过写入预检");
    let mut bytes = vec![0_u8; usize::try_from(length).expect("长度")];
    encode_object(input, FormatLimits::HARD, &mut bytes).expect("编码对象");
    bytes
}

/// 最小对象的逐表行：可构造的单例表恰一行，其余表零行。Genesis 基线的 LFSD
/// 另须在第六节新增恰一份空间配置。
fn minimal_rows(
    kind: PortableObjectKind,
    section: u16,
    table: &laneflow_static_contract::PortableTableSchema,
) -> Vec<RowWriteInput<'static>> {
    if (kind, section, table.kind) == (PortableObjectKind::SemanticDiff, 6, 1) {
        return vec![RowWriteInput {
            fields: leak_fields(vec![
                FieldWriteInput {
                    tag: 1,
                    value: FieldWriteValue::U8(0),
                },
                FieldWriteInput {
                    tag: 3,
                    value: FieldWriteValue::Bytes(&[1]),
                },
            ]),
        }];
    }
    default_rows(kind, section, table)
}

/// 构造某类对象的最小合法字节。
pub fn minimal_object(kind: PortableObjectKind) -> Vec<u8> {
    encode(kind, |section, table| minimal_rows(kind, section, table))
}

/// registry 结构合法、直接值域非法的对象。
pub struct ValueDomainMutation {
    /// 变异说明，用于断言消息。
    pub name: &'static str,
    /// 对象种类。
    pub kind: PortableObjectKind,
    /// 对象 exact bytes。
    pub bytes: Vec<u8>,
}

/// 在最小对象的单例行上把一个字段替换为给定值，不断言值域。
fn minimal_object_with_field(
    kind: PortableObjectKind,
    target: (u16, u16, u16),
    value: FieldWriteValue<'static>,
) -> Vec<u8> {
    encode_structure(kind, |section, table| {
        let mut rows = minimal_rows(kind, section, table);
        if (section, table.kind) == (target.0, target.1) {
            let row = rows.first_mut().expect("目标为可构造的单例表");
            let fields: Vec<FieldWriteInput<'static>> = row
                .fields
                .iter()
                .map(|field| {
                    if field.tag == target.2 {
                        FieldWriteInput {
                            tag: field.tag,
                            value,
                        }
                    } else {
                        *field
                    }
                })
                .collect();
            row.fields = leak_fields(fields);
        }
        rows
    })
}

/// 覆盖四类对象与封闭枚举、精确版本、对象键绑定、行号下界、修改不等式及
/// ASCII token 语法的值域负例；每份都能通过 registry 结构预检。
pub fn value_domain_mutations() -> Vec<ValueDomainMutation> {
    use PortableObjectKind as K;
    let mismatched_key: &'static str = Box::leak(format!("sha256/{}", "22".repeat(32)).into());
    vec![
        ValueDomainMutation {
            name: "LFCP 发布角色越出封闭枚举",
            kind: K::CanonicalPublicationDescriptor,
            bytes: minimal_object_with_field(
                K::CanonicalPublicationDescriptor,
                (3, 1, 1),
                FieldWriteValue::U8(255),
            ),
        },
        ValueDomainMutation {
            name: "LFCP 制品对象键与登记摘要不符",
            kind: K::CanonicalPublicationDescriptor,
            bytes: minimal_object_with_field(
                K::CanonicalPublicationDescriptor,
                (3, 1, 3),
                FieldWriteValue::Utf8(mismatched_key),
            ),
        },
        ValueDomainMutation {
            name: "LFCA 格式版本与对象种类不符",
            kind: K::CanonicalArtifact,
            bytes: minimal_object_with_field(
                K::CanonicalArtifact,
                (1, 1, 1),
                FieldWriteValue::U16(CANONICAL_ARTIFACT_FORMAT_VERSION + 1),
            ),
        },
        ValueDomainMutation {
            name: "LFCA 身份字段不是合法 ASCII token",
            kind: K::CanonicalArtifact,
            bytes: lfca_identity_bytes(&[b"ns", b"bad token"], false),
        },
        ValueDomainMutation {
            name: "LFSM 文本位置行号为零",
            kind: K::SourceMap,
            bytes: lfsm_source_location_bytes(&[0], false),
        },
        ValueDomainMutation {
            name: "LFSD 修改行前后值相同",
            kind: K::SemanticDiff,
            bytes: lfsd_evidence_change_bytes("k", "locator-a", "locator-a", false),
        },
    ]
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
    lfsm_source_location_bytes(start_lines, true)
}

fn lfsm_source_location_bytes(start_lines: &[u32], checked: bool) -> Vec<u8> {
    let kind = PortableObjectKind::SourceMap;
    build(kind, checked, |section, table| {
        if table.kind != 3 {
            return default_rows(kind, section, table);
        }
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
    })
}

/// 构造 kind 0（evidence）成员值的完整 RowV1 字节。
///
/// 测试数据按线格式文档手工装配（与 laneflow-format 自身测试的装配方式一致）：
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

/// 以 Artifact 基线（`baseKind = 1`）构造 LFSD 差异绑定行，使修改类变更合法。
fn artifact_base_binding_row(schema: &PortableRowSchema) -> RowWriteInput<'static> {
    use laneflow_static_contract::PortableFieldType as T;
    let kind = PortableObjectKind::SemanticDiff;
    let fields = schema
        .fields
        .iter()
        .filter(|field| field.presence == PortableFieldPresence::Required)
        .filter_map(|field| {
            let value = match (field.tag, field.field_type) {
                (1, _) => FieldWriteValue::U8(1),
                (2, _) => FieldWriteValue::U16(NETWORK_REVISION_DERIVATION_VERSION),
                (3 | 4, T::Sha256) => FieldWriteValue::Sha256([0x22; 32]),
                (3 | 4, T::StableId128) => FieldWriteValue::StableId128([0x22; 16]),
                (5, _) => FieldWriteValue::U64(1),
                _ => default_value(kind, 1, 1, field)?,
            };
            Some(FieldWriteInput {
                tag: field.tag,
                value,
            })
        })
        .collect();
    RowWriteInput {
        fields: leak_fields(fields),
    }
}

/// 构造带一条 PolicyLocalChange 修改行（kind 0 evidence 成员，before/after 各一份）的 LFSD。
pub fn lfsd_with_evidence_change(key: &str, before_locator: &str, after_locator: &str) -> Vec<u8> {
    lfsd_evidence_change_bytes(key, before_locator, after_locator, true)
}

fn lfsd_evidence_change_bytes(
    key: &str,
    before_locator: &str,
    after_locator: &str,
    checked: bool,
) -> Vec<u8> {
    let kind = PortableObjectKind::SemanticDiff;
    let key: &'static str = Box::leak(key.to_string().into_boxed_str());
    let member = |locator: &str| -> &'static [u8] {
        Box::leak(evidence_member_bytes(locator).into_boxed_slice())
    };
    let (before, after) = (member(before_locator), member(after_locator));
    build(kind, checked, |section, table| {
        match (section, table.kind) {
            (1, 1) => vec![artifact_base_binding_row(table.row)],
            (7, 1) => vec![RowWriteInput {
                fields: leak_fields(vec![
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
                        value: FieldWriteValue::Utf8(key),
                    },
                    FieldWriteInput {
                        tag: 5,
                        value: FieldWriteValue::Bytes(before),
                    },
                    FieldWriteInput {
                        tag: 6,
                        value: FieldWriteValue::Bytes(after),
                    },
                ]),
            }],
            _ => default_rows(kind, section, table),
        }
    })
}

/// 构造带一行 RoadCorridor CanonicalIdentity 的 LFCA。
///
/// `values` 依次是 `AuthoringNamespaceId` 与 `CorridorKey` 两个 ASCII token，
/// 作为 identityFields 嵌套行的 Bytes 值。
pub fn lfca_with_identity_field_values(values: &[&[u8]]) -> Vec<u8> {
    lfca_identity_bytes(values, true)
}

fn lfca_identity_bytes(values: &[&[u8]], checked: bool) -> Vec<u8> {
    let kind = PortableObjectKind::CanonicalArtifact;
    let entity = EntityKind::RoadCorridor;
    let tags = entity.required_tags();
    assert_eq!(values.len(), tags.len(), "RoadCorridor 身份恰有两项");
    build(kind, checked, |section, table| {
        if (section, table.kind) != (2, 1) {
            return default_rows(kind, section, table);
        }
        let nested: Vec<RowWriteInput<'static>> = values
            .iter()
            .zip(tags)
            .map(|(value, tag)| RowWriteInput {
                fields: leak_fields(vec![
                    FieldWriteInput {
                        tag: 1,
                        value: FieldWriteValue::U16(tag.code()),
                    },
                    FieldWriteInput {
                        tag: 2,
                        value: FieldWriteValue::Bytes(Box::leak(value.to_vec().into_boxed_slice())),
                    },
                ]),
            })
            .collect();
        vec![RowWriteInput {
            fields: leak_fields(vec![
                FieldWriteInput {
                    tag: 1,
                    value: FieldWriteValue::U16(entity.code()),
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
            ]),
        }]
    })
}

/// 构造带指定 `publisherBuildId` 的 LFCP（其余保持最小对象形状）。
pub fn lfcp_with_build_note(note: &str) -> Vec<u8> {
    lfcp_with_controlled_fields(note, None, None)
}

/// 构造带可控 provenance 可选字段的 LFCP（评审 #5/#7 复现锚）。
pub fn lfcp_with_controlled_fields(
    build_note: &str,
    provenance: Option<&str>,
    timestamp: Option<&str>,
) -> Vec<u8> {
    let kind = PortableObjectKind::CanonicalPublicationDescriptor;
    let leak = |text: &str| -> &'static str { Box::leak(text.to_string().into_boxed_str()) };
    let build_note = leak(build_note);
    let optional = [(5, provenance.map(leak)), (6, timestamp.map(leak))];
    encode(kind, |section, table| {
        let mut rows = default_rows(kind, section, table);
        if section != 3 {
            return rows;
        }
        let row = rows.first_mut().expect("LFCP 第三节为单例表");
        let mut fields: Vec<FieldWriteInput<'static>> = row
            .fields
            .iter()
            .map(|field| match field.tag {
                2 => FieldWriteInput {
                    tag: 2,
                    value: FieldWriteValue::Utf8(build_note),
                },
                _ => *field,
            })
            .collect();
        for (tag, text) in optional {
            if let Some(text) = text {
                fields.push(FieldWriteInput {
                    tag,
                    value: FieldWriteValue::Utf8(text),
                });
            }
        }
        row.fields = leak_fields(fields);
        rows
    })
}
