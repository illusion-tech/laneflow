//! 可移植规范制品的共享线格式登记值。
//!
//! 本模块只保存已经由 #298 G1 冻结的版本、magic、封闭字段类型、安全天花板和无分配
//! 值类型。字节读写、结构预检、编译器语义投影与后发射闭合检查分别属于它们自己的
//! crate；不得把本模块扩张成第二套 emitter 或语义验证算法。

use core::fmt;

/// LFCA 的对象格式版本。
pub const CANONICAL_ARTIFACT_FORMAT_VERSION: u16 = 1;

/// LFSM 的对象格式版本。
pub const SOURCE_MAP_FORMAT_VERSION: u16 = 1;

/// LFSD 的对象格式版本。
pub const SEMANTIC_DIFF_FORMAT_VERSION: u16 = 1;

/// LFCP 的对象格式版本；生产代码只接受无 receipt 的 v2。
pub const CANONICAL_PUBLICATION_DESCRIPTOR_VERSION: u16 = 2;

/// 六个 LFCA 语义节派生 [`NetworkRevisionId`] 的算法版本。
pub const NETWORK_REVISION_DERIVATION_VERSION: u16 = 1;

/// `NetworkRevisionId` SHA-256 输入的域分离前缀；末尾 NUL 是输入的一部分。
pub const NETWORK_REVISION_DOMAIN_PREFIX: &[u8] = b"laneflow.network-revision.v1\0";

/// `ObjectPreambleV1` 的固定字节长度。
pub const OBJECT_PREAMBLE_V1_BYTE_LENGTH: u16 = 32;

/// `SectionDirectoryEntryV1` 的固定字节长度。
pub const SECTION_DIRECTORY_ENTRY_V1_BYTE_LENGTH: u64 = 24;

/// v1 所有节的格式版本。
pub const SECTION_FORMAT_VERSION_V1: u16 = 1;

/// 单对象 exact bytes 的格式安全天花板。
pub const FORMAT_HARD_MAX_OBJECT_BYTES: u64 = 16_777_216;

/// 单节或单表 exact bytes 的格式安全天花板。
pub const FORMAT_HARD_MAX_SECTION_OR_TABLE_BYTES: u64 = 16_777_216;

/// 单 TableV1 的 RowV1 数量安全天花板。
pub const FORMAT_HARD_MAX_ROWS_PER_TABLE: u32 = 65_536;

/// 单 RowV1 的 FieldV1 数量安全天花板。
pub const FORMAT_HARD_MAX_FIELDS_PER_ROW: u32 = 17;

/// Identity v1 `Ascii` 值的最大字节数。
pub const FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES: u64 = 53;

/// 单 UTF-8 FieldV1 value 的最大字节数。
pub const FORMAT_HARD_MAX_UTF8_FIELD_BYTES: u64 = 1_048_576;

/// 单对象全部 UTF-8 value 的累计字节安全天花板。
pub const FORMAT_HARD_MAX_TOTAL_UTF8_BYTES: u64 = 8_388_608;

/// 单向量的最大 item 数。
pub const FORMAT_HARD_MAX_VECTOR_ITEMS: u32 = 65_536;

/// 单对象全部 vector value 的累计字节安全天花板。
pub const FORMAT_HARD_MAX_TOTAL_VECTOR_BYTES: u64 = 8_388_608;

/// `RecordVector` 允许的一层内嵌深度。
pub const FORMAT_HARD_MAX_RECORD_VECTOR_DEPTH: u8 = 1;

/// 单 LFSM 的来源位置记录数安全天花板。
pub const FORMAT_HARD_MAX_SOURCE_LOCATION_ROWS: u32 = 65_536;

/// 单次 LFCA + LFSM + LFSD 候选暂存 exact bytes 总预算（48 MiB）。
pub const FORMAT_HARD_MAX_CANDIDATE_STAGING_BYTES: u64 = 50_331_648;

/// 可移植规范制品对象种类。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PortableObjectKind {
    /// 可移植规范制品 `LFCA`。
    CanonicalArtifact,
    /// 源映射封套 `LFSM`。
    SourceMap,
    /// 语义差异封套 `LFSD`。
    SemanticDiff,
    /// 规范发布描述符 `LFCP`。
    CanonicalPublicationDescriptor,
}

impl PortableObjectKind {
    /// 按 wire magic 排列的四种当前对象。
    pub const ALL: [Self; 4] = [
        Self::CanonicalArtifact,
        Self::SourceMap,
        Self::SemanticDiff,
        Self::CanonicalPublicationDescriptor,
    ];

    /// 对象前导中的四字节 ASCII magic。
    #[must_use]
    pub const fn magic(self) -> [u8; 4] {
        match self {
            Self::CanonicalArtifact => *b"LFCA",
            Self::SourceMap => *b"LFSM",
            Self::SemanticDiff => *b"LFSD",
            Self::CanonicalPublicationDescriptor => *b"LFCP",
        }
    }

    /// 对象自己的当前格式版本。
    #[must_use]
    pub const fn format_version(self) -> u16 {
        match self {
            Self::CanonicalArtifact => CANONICAL_ARTIFACT_FORMAT_VERSION,
            Self::SourceMap => SOURCE_MAP_FORMAT_VERSION,
            Self::SemanticDiff => SEMANTIC_DIFF_FORMAT_VERSION,
            Self::CanonicalPublicationDescriptor => CANONICAL_PUBLICATION_DESCRIPTOR_VERSION,
        }
    }

    /// 当前对象必须拥有的精确节数。
    #[must_use]
    pub const fn section_count(self) -> u32 {
        match self {
            Self::CanonicalArtifact => 8,
            Self::SourceMap => 5,
            Self::SemanticDiff => 6,
            Self::CanonicalPublicationDescriptor => 3,
        }
    }

    /// 当前对象登记的精确 TableV1 总数。
    #[must_use]
    pub const fn table_count(self) -> u32 {
        match self {
            Self::CanonicalArtifact => 35,
            Self::SourceMap => 8,
            Self::SemanticDiff => 6,
            Self::CanonicalPublicationDescriptor => 3,
        }
    }

    /// 第一节的规范 wire offset，即 `32 + sectionCount * 24`。
    #[must_use]
    pub const fn first_section_offset(self) -> u64 {
        OBJECT_PREAMBLE_V1_BYTE_LENGTH as u64
            + self.section_count() as u64 * SECTION_DIRECTORY_ENTRY_V1_BYTE_LENGTH
    }

    /// 从四字节 magic 解析封闭对象种类。
    #[must_use]
    pub const fn from_magic(magic: [u8; 4]) -> Option<Self> {
        if magic[0] == b'L' && magic[1] == b'F' && magic[2] == b'C' && magic[3] == b'A' {
            Some(Self::CanonicalArtifact)
        } else if magic[0] == b'L' && magic[1] == b'F' && magic[2] == b'S' && magic[3] == b'M' {
            Some(Self::SourceMap)
        } else if magic[0] == b'L' && magic[1] == b'F' && magic[2] == b'S' && magic[3] == b'D' {
            Some(Self::SemanticDiff)
        } else if magic[0] == b'L' && magic[1] == b'F' && magic[2] == b'C' && magic[3] == b'P' {
            Some(Self::CanonicalPublicationDescriptor)
        } else {
            None
        }
    }
}

/// `FieldV1.fieldType` 的封闭 v1 登记表。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum PortableFieldType {
    U8 = 1,
    U16 = 2,
    U32 = 3,
    U64 = 4,
    F32 = 5,
    F64 = 6,
    StableId128 = 7,
    Sha256 = 8,
    Utf8 = 9,
    Bytes = 10,
    OrdinalVectorU32 = 11,
    RecordVector = 12,
    I32 = 13,
}

impl PortableFieldType {
    /// 按 wire code 递增的全部 v1 字段类型。
    pub const ALL: [Self; 13] = [
        Self::U8,
        Self::U16,
        Self::U32,
        Self::U64,
        Self::F32,
        Self::F64,
        Self::StableId128,
        Self::Sha256,
        Self::Utf8,
        Self::Bytes,
        Self::OrdinalVectorU32,
        Self::RecordVector,
        Self::I32,
    ];

    /// 解析封闭字段类型；未知 code 返回 `None`。
    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::U8),
            2 => Some(Self::U16),
            3 => Some(Self::U32),
            4 => Some(Self::U64),
            5 => Some(Self::F32),
            6 => Some(Self::F64),
            7 => Some(Self::StableId128),
            8 => Some(Self::Sha256),
            9 => Some(Self::Utf8),
            10 => Some(Self::Bytes),
            11 => Some(Self::OrdinalVectorU32),
            12 => Some(Self::RecordVector),
            13 => Some(Self::I32),
            _ => None,
        }
    }

    /// 固定宽度类型的 exact value bytes；变长类型返回 `None`。
    #[must_use]
    pub const fn fixed_width(self) -> Option<u64> {
        match self {
            Self::U8 => Some(1),
            Self::U16 => Some(2),
            Self::U32 | Self::F32 | Self::I32 => Some(4),
            Self::U64 | Self::F64 => Some(8),
            Self::StableId128 => Some(16),
            Self::Sha256 => Some(32),
            Self::Utf8 | Self::Bytes | Self::OrdinalVectorU32 | Self::RecordVector => None,
        }
    }
}

/// 32-byte SHA-256 原始摘要值。
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct Sha256Digest([u8; 32]);

impl Sha256Digest {
    /// 全零值只用于明确允许零绑定的协议位置，不表示已验证摘要。
    pub const ZERO: Self = Self([0; 32]);

    /// 原样封装 32 个摘要字节，不执行 hash 或信任验证。
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// 借用摘要原始字节。
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// 取出摘要原始字节。
    #[must_use]
    pub const fn into_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::LowerHex for Sha256Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Sha256Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Sha256Digest({self:x})")
    }
}

/// 由六个 LFCA 语义节独立派生的 32-byte 路网修订标识。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct NetworkRevisionId(Sha256Digest);

impl NetworkRevisionId {
    /// 从已按冻结算法计算的摘要建立值类型；本函数不执行重算。
    #[must_use]
    pub const fn from_digest(digest: Sha256Digest) -> Self {
        Self(digest)
    }

    /// 借用底层 SHA-256 值。
    #[must_use]
    pub const fn as_digest(&self) -> &Sha256Digest {
        &self.0
    }

    /// 取出底层 SHA-256 值。
    #[must_use]
    pub const fn into_digest(self) -> Sha256Digest {
        self.0
    }
}

/// 与摘要共同绑定对象 exact bytes 的无符号 64-bit 长度值。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct ExactByteLength(u64);

impl ExactByteLength {
    /// 原样封装受检长度；具体对象的非零与上限规则由格式边界执行。
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// 返回原始 `u64` 长度。
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use core::mem::size_of;

    use super::*;

    #[test]
    fn object_registry_matches_g1_offsets_and_shapes() {
        let expected = [
            (
                PortableObjectKind::CanonicalArtifact,
                *b"LFCA",
                8,
                35,
                0x00e0,
            ),
            (PortableObjectKind::SourceMap, *b"LFSM", 5, 8, 0x0098),
            (PortableObjectKind::SemanticDiff, *b"LFSD", 6, 6, 0x00b0),
            (
                PortableObjectKind::CanonicalPublicationDescriptor,
                *b"LFCP",
                3,
                3,
                0x0068,
            ),
        ];

        assert_eq!(PortableObjectKind::ALL.len(), expected.len());
        for (actual, (kind, magic, sections, tables, first_offset)) in
            PortableObjectKind::ALL.into_iter().zip(expected)
        {
            assert_eq!(actual, kind);
            assert_eq!(kind.magic(), magic);
            assert_eq!(PortableObjectKind::from_magic(magic), Some(kind));
            let expected_version = if kind == PortableObjectKind::CanonicalPublicationDescriptor {
                2
            } else {
                1
            };
            assert_eq!(kind.format_version(), expected_version);
            assert_eq!(kind.section_count(), sections);
            assert_eq!(kind.table_count(), tables);
            assert_eq!(kind.first_section_offset(), first_offset);
        }
        assert_eq!(PortableObjectKind::from_magic(*b"LFXX"), None);
    }

    #[test]
    fn field_type_registry_is_closed_and_has_exact_widths() {
        let expected_widths = [
            Some(1),
            Some(2),
            Some(4),
            Some(8),
            Some(4),
            Some(8),
            Some(16),
            Some(32),
            None,
            None,
            None,
            None,
            Some(4),
        ];

        for (index, field_type) in PortableFieldType::ALL.into_iter().enumerate() {
            let code = u8::try_from(index + 1).unwrap();
            assert_eq!(field_type as u8, code);
            assert_eq!(PortableFieldType::from_code(code), Some(field_type));
            assert_eq!(field_type.fixed_width(), expected_widths[index]);
        }
        assert_eq!(PortableFieldType::from_code(0), None);
        assert_eq!(PortableFieldType::from_code(14), None);
    }

    #[test]
    fn digest_and_length_types_preserve_wire_sizes() {
        assert_eq!(size_of::<Sha256Digest>(), 32);
        assert_eq!(size_of::<NetworkRevisionId>(), 32);
        assert_eq!(size_of::<ExactByteLength>(), 8);
    }

    #[test]
    fn hard_limits_match_g1_contract() {
        assert_eq!(FORMAT_HARD_MAX_OBJECT_BYTES, 16_777_216);
        assert_eq!(FORMAT_HARD_MAX_SECTION_OR_TABLE_BYTES, 16_777_216);
        assert_eq!(FORMAT_HARD_MAX_ROWS_PER_TABLE, 65_536);
        assert_eq!(FORMAT_HARD_MAX_FIELDS_PER_ROW, 17);
        assert_eq!(FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES, 53);
        assert_eq!(FORMAT_HARD_MAX_UTF8_FIELD_BYTES, 1_048_576);
        assert_eq!(FORMAT_HARD_MAX_TOTAL_UTF8_BYTES, 8_388_608);
        assert_eq!(FORMAT_HARD_MAX_VECTOR_ITEMS, 65_536);
        assert_eq!(FORMAT_HARD_MAX_TOTAL_VECTOR_BYTES, 8_388_608);
        assert_eq!(FORMAT_HARD_MAX_RECORD_VECTOR_DEPTH, 1);
        assert_eq!(FORMAT_HARD_MAX_SOURCE_LOCATION_ROWS, 65_536);
        assert_eq!(FORMAT_HARD_MAX_CANDIDATE_STAGING_BYTES, 50_331_648);
    }
}
