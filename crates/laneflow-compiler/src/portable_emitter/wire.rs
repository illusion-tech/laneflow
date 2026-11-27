use super::*;

/// emitter 内部持有的拥有型字段值：标量、字符串、字节、ordinal 向量或一层记录向量。
#[derive(Clone, Debug, PartialEq)]
pub(super) enum OwnedValue {
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    F32(f32),
    /// 供测试侧通用字段复制与线格式枚举映射使用。
    F64(f64),
    StableId128([u8; 16]),
    Sha256([u8; 32]),
    Utf8(Box<str>),
    Bytes(Box<[u8]>),
    OrdinalVectorU32(Box<[u32]>),
    RecordVector(Box<[OwnedRow]>),
    I32(i32),
}

/// 带 tag 的拥有型字段。
#[derive(Clone, Debug, PartialEq)]
pub(super) struct OwnedField {
    pub(super) tag: u16,
    pub(super) value: OwnedValue,
}

/// 拥有型行：一组有序字段。
#[derive(Clone, Debug, PartialEq)]
pub(super) struct OwnedRow {
    pub(super) fields: Box<[OwnedField]>,
}

/// 拥有型表：表 kind 与有序行。
#[derive(Clone, Debug, PartialEq)]
pub(super) struct OwnedTable {
    pub(super) kind: u16,
    pub(super) rows: Box<[OwnedRow]>,
}

/// 拥有型 section：section kind 与有序表。
#[derive(Clone, Debug, PartialEq)]
pub(super) struct OwnedSection {
    pub(super) kind: u16,
    pub(super) tables: Box<[OwnedTable]>,
}

/// 拥有型 object：可移植对象 kind 与有序 section，是编码前的完整 emitter 投影。
#[derive(Clone, Debug, PartialEq)]
pub(super) struct OwnedObject {
    pub(super) kind: PortableObjectKind,
    pub(super) sections: Box<[OwnedSection]>,
}

/// 构造一个拥有型字段。
pub(super) fn field(tag: u16, value: OwnedValue) -> OwnedField {
    OwnedField { tag, value }
}

/// 从字段迭代器构造拥有型行。
pub(super) fn row(fields: impl IntoIterator<Item = OwnedField>) -> OwnedRow {
    OwnedRow {
        fields: fields.into_iter().collect(),
    }
}

/// 从行迭代器构造拥有型表。
pub(super) fn table(kind: u16, rows: impl IntoIterator<Item = OwnedRow>) -> OwnedTable {
    OwnedTable {
        kind,
        rows: rows.into_iter().collect(),
    }
}

/// 从表迭代器构造拥有型 section。
pub(super) fn section(kind: u16, tables: impl IntoIterator<Item = OwnedTable>) -> OwnedSection {
    OwnedSection {
        kind,
        tables: tables.into_iter().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_lfcp_shape() -> OwnedObject {
        OwnedObject {
            kind: PortableObjectKind::CanonicalPublicationDescriptor,
            sections: vec![
                section(
                    1,
                    [table(
                        1,
                        [row([
                            field(1, OwnedValue::U16(1)),
                            field(2, OwnedValue::U16(1)),
                            field(3, OwnedValue::Sha256([1; 32])),
                            field(4, OwnedValue::Sha256([2; 32])),
                            field(5, OwnedValue::U64(1)),
                        ])],
                    )],
                ),
                section(
                    2,
                    [table(
                        1,
                        [row([
                            field(1, OwnedValue::U16(1)),
                            field(2, OwnedValue::Sha256([3; 32])),
                            field(3, OwnedValue::U64(1)),
                            field(4, OwnedValue::Utf8("compiler".into())),
                            field(5, OwnedValue::U16(1)),
                            field(6, OwnedValue::Sha256([4; 32])),
                        ])],
                    )],
                ),
                section(
                    3,
                    [table(
                        1,
                        [row([
                            field(1, OwnedValue::U8(1)),
                            field(2, OwnedValue::Utf8("publisher".into())),
                            field(3, OwnedValue::Utf8("artifact-key".into())),
                            field(4, OwnedValue::Utf8("source-map-key".into())),
                        ])],
                    )],
                ),
            ]
            .into_boxed_slice(),
        }
    }

    #[test]
    fn complete_object_is_not_misclassified_as_staged_chunk_scratch() {
        let object = minimal_lfcp_shape();
        let length = encode_owned_object(&object, FormatLimits::HARD)
            .unwrap()
            .len() as u64;
        let mut config = laneflow_format::FormatLimitConfig::HARD;
        config.max_staged_chunk_bytes = length - 1;
        let encoded = encode_owned_object(&object, FormatLimits::try_new(config).unwrap()).unwrap();
        assert_eq!(encoded.len() as u64, length);
    }
}
