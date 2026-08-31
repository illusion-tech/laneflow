//! 完整对象的附录 A registry 结构预检。

use laneflow_static_contract::{
    CHUNKED_SECTION_PREAMBLE_BYTE_LENGTH, PortableFieldSchema, PortableFieldType,
    PortableObjectKind, PortableObjectSchema, PortableRowCardinality, PortableRowSchema,
    PortableSectionSchema, PortableTableSchema, Sha256Digest, StableId128,
    TABLE_CHUNK_DIRECTORY_ENTRY_BYTE_LENGTH, portable_object_schema,
};
use sha2::{Digest, Sha256};

use crate::{
    FormatError, FormatLimits, FormatStructure, LimitDimension, ObjectFramingView,
    SectionFramingView,
    framing::ObjectFramingProof,
    limits::{CanonicalChunkMetrics, canonical_chunk_with_appended_row},
    table::{PreflightBudget, preflight_table_with_registry},
    wire::{checked_slice, read_u8, read_u16, read_u32, read_u64},
};

const TABLE_HEADER_BYTES: u64 = 16;
const ROW_HEADER_BYTES: u64 = 16;
const FIELD_HEADER_BYTES: u64 = 12;

/// 已完成前导、目录和附录 A section/table/field registry 结构预检的对象借用。
///
/// 本类型只证明闭合结构形状、冗余长度/计数和通用值编码；它不证明行排序键、跨表引用、
/// 摘要绑定、NetworkRevision、语义差异完备性或发布真实性。不得把它包装为
/// validated/trusted artifact view。
#[derive(Clone, Copy, Debug)]
pub struct RegistryCheckedObjectView<'a> {
    framing: ObjectFramingView<'a>,
    limits: FormatLimits,
    schema: &'static PortableObjectSchema,
}

/// 成功 registry 预检后可对同一不可变 backing 做 O(1) 重借用的 crate-private 证明。
#[derive(Clone, Copy, Debug)]
pub(crate) struct RegistryCheckProof {
    framing: ObjectFramingProof,
    limits: FormatLimits,
    schema: &'static PortableObjectSchema,
}

impl<'a> RegistryCheckedObjectView<'a> {
    pub(crate) const fn proof(self) -> RegistryCheckProof {
        RegistryCheckProof {
            framing: self.framing.proof(),
            limits: self.limits,
            schema: self.schema,
        }
    }

    /// 已与 magic、目录和静态 registry 一致的对象种类。
    #[must_use]
    pub const fn kind(self) -> PortableObjectKind {
        self.framing.kind()
    }

    /// 整个对象的 exact bytes；其中每个 section/table/field 都已通过 registry 结构预检。
    #[must_use]
    pub const fn bytes(self) -> &'a [u8] {
        self.framing.bytes()
    }

    pub(crate) const fn limits(self) -> FormatLimits {
        self.limits
    }

    pub(crate) const fn schema(self) -> &'static PortableObjectSchema {
        self.schema
    }

    /// 附录 A 冻结的精确 section 数量。
    #[must_use]
    pub const fn section_count(self) -> u32 {
        self.framing.section_count()
    }

    /// 按 wire 顺序遍历全部已受检 section。
    #[must_use]
    pub fn sections(self) -> RegistryCheckedSectionIter<'a> {
        RegistryCheckedSectionIter {
            object: self,
            ordinal: 0,
        }
    }

    /// 取得一个已经完成节内 registry 结构预检的 section。
    #[must_use]
    pub fn section(self, ordinal: u32) -> Option<RegistryCheckedSectionView<'a>> {
        let schema = self.schema.sections.get(usize::try_from(ordinal).ok()?)?;
        let framing = self.framing.section(ordinal)?;
        Some(RegistryCheckedSectionView {
            framing,
            schema,
            chunked: self.kind() != PortableObjectKind::CanonicalPublicationDescriptor,
        })
    }
}

impl RegistryCheckProof {
    pub(crate) fn reborrow(self, bytes: &[u8]) -> Option<RegistryCheckedObjectView<'_>> {
        Some(RegistryCheckedObjectView {
            framing: self.framing.reborrow(bytes)?,
            limits: self.limits,
            schema: self.schema,
        })
    }
}

/// 已完成该节全部 table/row/field registry 结构预检的借用。
#[derive(Clone, Copy, Debug)]
pub struct RegistryCheckedSectionView<'a> {
    framing: SectionFramingView<'a>,
    schema: &'static PortableSectionSchema,
    chunked: bool,
}

impl<'a> RegistryCheckedSectionView<'a> {
    #[must_use]
    pub const fn kind(self) -> u16 {
        self.framing.kind()
    }

    #[must_use]
    pub const fn bytes(self) -> &'a [u8] {
        self.framing.bytes()
    }

    /// 附录 A 为该节冻结的 table 数量。
    #[must_use]
    pub const fn table_count(self) -> u32 {
        self.schema.tables.len() as u32
    }

    /// 按 wire 顺序遍历全部已受检 table；每张 table 只推进一次游标。
    #[must_use]
    pub fn tables(self) -> RegistryCheckedTableIter<'a> {
        RegistryCheckedTableIter {
            section: self,
            ordinal: 0,
        }
    }

    /// 按附录 A 顺序取得一张已完成 registry 预检的 table。
    ///
    /// 本随机访问兼容 API 从 section 起点扫描，复杂度为 O(`ordinal`)；顺序消费应使用
    /// [`Self::tables`]。
    #[must_use]
    pub fn table(self, ordinal: u32) -> Option<RegistryCheckedTableView<'a>> {
        let ordinal = usize::try_from(ordinal).ok()?;
        let schema = self.schema.tables.get(ordinal)?;
        if !self.chunked {
            let bytes = self.bytes();
            let row_count = read_u32(bytes, 4, FormatStructure::Table).ok()?;
            return Some(RegistryCheckedTableView {
                section_bytes: bytes,
                directory: &[],
                first_chunk: 0,
                chunk_count: 1,
                schema,
                row_count,
                chunked: false,
            });
        }

        let chunk_count = read_u32(self.bytes(), 0, FormatStructure::Section).ok()?;
        let directory = checked_slice(
            self.bytes(),
            CHUNKED_SECTION_PREAMBLE_BYTE_LENGTH,
            u64::from(chunk_count) * TABLE_CHUNK_DIRECTORY_ENTRY_BYTE_LENGTH,
            FormatStructure::ChunkDirectory,
        )
        .ok()?;
        let mut first_chunk = None;
        let mut matching_chunks = 0_u32;
        let mut row_count = 0_u32;
        for entry in 0..chunk_count {
            let offset = u64::from(entry) * TABLE_CHUNK_DIRECTORY_ENTRY_BYTE_LENGTH;
            let table_kind =
                read_u16(directory, offset, FormatStructure::ChunkDirectoryEntry).ok()?;
            if table_kind == schema.kind {
                first_chunk.get_or_insert(entry);
                matching_chunks += 1;
                row_count = row_count.checked_add(
                    read_u32(directory, offset + 12, FormatStructure::ChunkDirectoryEntry).ok()?,
                )?;
            } else if table_kind > schema.kind && first_chunk.is_some() {
                break;
            }
        }
        Some(RegistryCheckedTableView {
            section_bytes: self.bytes(),
            directory,
            first_chunk: first_chunk.unwrap_or(0),
            chunk_count: matching_chunks,
            schema,
            row_count,
            chunked: true,
        })
    }
}

/// 已按附录 A table schema 完成结构预检的零拷贝借用。
#[derive(Clone, Copy, Debug)]
pub struct RegistryCheckedTableView<'a> {
    section_bytes: &'a [u8],
    directory: &'a [u8],
    first_chunk: u32,
    chunk_count: u32,
    schema: &'static PortableTableSchema,
    row_count: u32,
    chunked: bool,
}

impl<'a> RegistryCheckedTableView<'a> {
    #[must_use]
    pub const fn kind(self) -> u16 {
        self.schema.kind
    }

    #[must_use]
    pub const fn row_count(self) -> u32 {
        self.row_count
    }

    /// 构成该逻辑表的物理 chunk 数；LFCP singleton table 固定为一。
    #[must_use]
    pub const fn chunk_count(self) -> u32 {
        self.chunk_count
    }

    /// 指定物理 chunk 的逻辑行数。
    #[must_use]
    pub fn chunk_row_count(self, chunk_ordinal: u32) -> Option<u32> {
        if chunk_ordinal >= self.chunk_count {
            return None;
        }
        if !self.chunked {
            return Some(self.row_count);
        }
        let entry = self.first_chunk.checked_add(chunk_ordinal)?;
        read_u32(
            self.directory,
            u64::from(entry) * TABLE_CHUNK_DIRECTORY_ENTRY_BYTE_LENGTH + 12,
            FormatStructure::ChunkDirectoryEntry,
        )
        .ok()
    }

    /// 指定物理 chunk 的 exact bytes。
    #[must_use]
    pub fn chunk_exact_byte_length(self, chunk_ordinal: u32) -> Option<u64> {
        if chunk_ordinal >= self.chunk_count {
            return None;
        }
        if !self.chunked {
            return u64::try_from(self.section_bytes.len()).ok();
        }
        let entry = self.first_chunk.checked_add(chunk_ordinal)?;
        read_u64(
            self.directory,
            u64::from(entry) * TABLE_CHUNK_DIRECTORY_ENTRY_BYTE_LENGTH + 32,
            FormatStructure::ChunkDirectoryEntry,
        )
        .ok()
    }

    /// 按 wire 顺序遍历全部已受检 row；每行只推进一次游标。
    #[must_use]
    pub fn rows(self) -> RegistryCheckedRowIter<'a> {
        RegistryCheckedRowIter::for_table(self)
    }

    /// 按线顺序取得一行。越过已受检的 row count 时返回 `None`。
    ///
    /// 本随机访问兼容 API 从 table 起点扫描，复杂度为 O(`ordinal`)；顺序消费应使用
    /// [`Self::rows`]。
    #[must_use]
    pub fn row(self, ordinal: u32) -> Option<RegistryCheckedRowView<'a>> {
        if ordinal >= self.row_count() {
            return None;
        }
        let (bytes, row_in_chunk) = self.chunk_for_row(ordinal)?;
        let mut cursor = TABLE_HEADER_BYTES;
        for current in 0..=row_in_chunk {
            let row_byte_length = read_u64(bytes, cursor, FormatStructure::Row).ok()?;
            if current == row_in_chunk {
                let bytes =
                    checked_slice(bytes, cursor, row_byte_length, FormatStructure::Row).ok()?;
                let field_count = read_u32(bytes, 8, FormatStructure::Row).ok()?;
                return Some(RegistryCheckedRowView {
                    bytes,
                    schema: self.schema.row,
                    field_count,
                });
            }
            cursor = cursor.checked_add(row_byte_length)?;
        }
        None
    }

    fn chunk_bytes(self, chunk_ordinal: u32) -> Option<&'a [u8]> {
        if chunk_ordinal >= self.chunk_count {
            return None;
        }
        if !self.chunked {
            return Some(self.section_bytes);
        }
        let entry = self.first_chunk.checked_add(chunk_ordinal)?;
        let offset = u64::from(entry) * TABLE_CHUNK_DIRECTORY_ENTRY_BYTE_LENGTH;
        let byte_offset = read_u64(
            self.directory,
            offset + 24,
            FormatStructure::ChunkDirectoryEntry,
        )
        .ok()?;
        let byte_length = read_u64(
            self.directory,
            offset + 32,
            FormatStructure::ChunkDirectoryEntry,
        )
        .ok()?;
        checked_slice(
            self.section_bytes,
            byte_offset,
            byte_length,
            FormatStructure::Table,
        )
        .ok()
    }

    fn chunk_for_row(self, ordinal: u32) -> Option<(&'a [u8], u32)> {
        if !self.chunked {
            return Some((self.section_bytes, ordinal));
        }
        for chunk in 0..self.chunk_count {
            let entry = self.first_chunk.checked_add(chunk)?;
            let offset = u64::from(entry) * TABLE_CHUNK_DIRECTORY_ENTRY_BYTE_LENGTH;
            let first = read_u32(
                self.directory,
                offset + 8,
                FormatStructure::ChunkDirectoryEntry,
            )
            .ok()?;
            let count = read_u32(
                self.directory,
                offset + 12,
                FormatStructure::ChunkDirectoryEntry,
            )
            .ok()?;
            if ordinal >= first && ordinal < first.checked_add(count)? {
                return Some((self.chunk_bytes(chunk)?, ordinal - first));
            }
        }
        None
    }
}

/// 已按 field tag/type/presence schema 完成结构预检的一行。
#[derive(Clone, Copy, Debug)]
pub struct RegistryCheckedRowView<'a> {
    bytes: &'a [u8],
    schema: &'static PortableRowSchema,
    field_count: u32,
}

impl<'a> RegistryCheckedRowView<'a> {
    #[must_use]
    pub const fn bytes(self) -> &'a [u8] {
        self.bytes
    }

    #[must_use]
    pub const fn field_count(self) -> u32 {
        self.field_count
    }

    /// 按 wire 顺序遍历全部已登记 field；每个字段只推进一次游标。
    #[must_use]
    pub fn fields(self) -> RegistryCheckedFieldIter<'a> {
        RegistryCheckedFieldIter {
            bytes: self.bytes,
            schemas: self.schema.fields,
            schema_index: 0,
            cursor: ROW_HEADER_BYTES,
            remaining: self.field_count(),
        }
    }

    /// 按线顺序取得一个已登记字段。
    ///
    /// 本随机访问兼容 API 从 row 起点扫描，复杂度为 O(`ordinal`)；顺序消费应使用
    /// [`Self::fields`]。
    #[must_use]
    pub fn field(self, ordinal: u32) -> Option<RegistryCheckedFieldView<'a>> {
        if ordinal >= self.field_count() {
            return None;
        }
        let mut cursor = ROW_HEADER_BYTES;
        for current in 0..=ordinal {
            let tag = read_u16(self.bytes, cursor, FormatStructure::Field).ok()?;
            let schema = self.schema.fields.iter().find(|field| field.tag == tag)?;
            let value_byte_length =
                read_u64(self.bytes, cursor + 4, FormatStructure::Field).ok()?;
            let field_byte_length = FIELD_HEADER_BYTES.checked_add(value_byte_length)?;
            if current == ordinal {
                let bytes = checked_slice(
                    self.bytes,
                    cursor,
                    field_byte_length,
                    FormatStructure::Field,
                )
                .ok()?;
                let value = checked_slice(
                    bytes,
                    FIELD_HEADER_BYTES,
                    value_byte_length,
                    FormatStructure::FieldValue,
                )
                .ok()?;
                return Some(RegistryCheckedFieldView { value, schema });
            }
            cursor = cursor.checked_add(field_byte_length)?;
        }
        None
    }

    /// 按稳定 field tag 查找字段；缺失 optional 字段时返回 `None`。
    #[must_use]
    pub fn field_by_tag(self, tag: u16) -> Option<RegistryCheckedFieldView<'a>> {
        self.fields().find(|field| field.tag() == tag)
    }
}

/// 已按登记 field type 完成通用编码预检的字段。
#[derive(Clone, Copy, Debug)]
pub struct RegistryCheckedFieldView<'a> {
    value: &'a [u8],
    schema: &'static PortableFieldSchema,
}

impl<'a> RegistryCheckedFieldView<'a> {
    #[must_use]
    pub const fn tag(self) -> u16 {
        self.schema.tag
    }

    #[must_use]
    pub const fn field_type(self) -> PortableFieldType {
        self.schema.field_type
    }

    #[must_use]
    pub const fn value_bytes(self) -> &'a [u8] {
        self.value
    }

    /// 按 registry 中已经证明的 field type 解码零拷贝值。
    pub fn value(self) -> Result<RegistryCheckedFieldValue<'a>, FormatError> {
        let value = self.value_bytes();
        Ok(match self.field_type() {
            PortableFieldType::U8 => {
                RegistryCheckedFieldValue::U8(read_u8(value, 0, FormatStructure::FieldValue)?)
            }
            PortableFieldType::U16 => {
                RegistryCheckedFieldValue::U16(read_u16(value, 0, FormatStructure::FieldValue)?)
            }
            PortableFieldType::U32 => {
                RegistryCheckedFieldValue::U32(read_u32(value, 0, FormatStructure::FieldValue)?)
            }
            PortableFieldType::U64 => {
                RegistryCheckedFieldValue::U64(read_u64(value, 0, FormatStructure::FieldValue)?)
            }
            PortableFieldType::F32 => RegistryCheckedFieldValue::F32(f32::from_bits(read_u32(
                value,
                0,
                FormatStructure::FieldValue,
            )?)),
            PortableFieldType::F64 => RegistryCheckedFieldValue::F64(f64::from_bits(read_u64(
                value,
                0,
                FormatStructure::FieldValue,
            )?)),
            PortableFieldType::StableId128 => RegistryCheckedFieldValue::StableId128(
                StableId128::from_bytes(read_array_value(value)?),
            ),
            PortableFieldType::Sha256 => RegistryCheckedFieldValue::Sha256(
                Sha256Digest::from_bytes(read_array_value(value)?),
            ),
            PortableFieldType::Utf8 => {
                RegistryCheckedFieldValue::Utf8(core::str::from_utf8(value).map_err(|_| {
                    FormatError::NonCanonicalValue {
                        structure: FormatStructure::FieldValue,
                        offset: 0,
                    }
                })?)
            }
            PortableFieldType::Bytes => RegistryCheckedFieldValue::Bytes(value),
            PortableFieldType::OrdinalVectorU32 => {
                let count = read_u32(value, 0, FormatStructure::OrdinalVector)?;
                RegistryCheckedFieldValue::OrdinalVectorU32(RegistryCheckedOrdinalVectorView {
                    bytes: value,
                    count,
                })
            }
            PortableFieldType::RecordVector => {
                let row_schema = self.schema.nested_row.ok_or(FormatError::BindingMismatch {
                    structure: FormatStructure::RecordVector,
                })?;
                let count = read_u32(value, 0, FormatStructure::RecordVector)?;
                RegistryCheckedFieldValue::RecordVector(RegistryCheckedRecordVectorView {
                    bytes: value,
                    row_schema,
                    count,
                })
            }
            PortableFieldType::I32 => {
                RegistryCheckedFieldValue::I32(i32::from_le_bytes(read_array_value(value)?))
            }
        })
    }
}

/// registry-checked 字段的零拷贝有类型值。
#[derive(Clone, Copy, Debug)]
pub enum RegistryCheckedFieldValue<'a> {
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    F32(f32),
    F64(f64),
    StableId128(StableId128),
    Sha256(Sha256Digest),
    Utf8(&'a str),
    Bytes(&'a [u8]),
    OrdinalVectorU32(RegistryCheckedOrdinalVectorView<'a>),
    RecordVector(RegistryCheckedRecordVectorView<'a>),
    I32(i32),
}

/// 已完成 count/length/limit 预检的 `OrdinalVectorU32`。
#[derive(Clone, Copy, Debug)]
pub struct RegistryCheckedOrdinalVectorView<'a> {
    bytes: &'a [u8],
    count: u32,
}

impl RegistryCheckedOrdinalVectorView<'_> {
    #[must_use]
    pub const fn len(self) -> u32 {
        self.count
    }

    #[must_use]
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub fn get(self, index: u32) -> Option<u32> {
        if index >= self.len() {
            return None;
        }
        let offset = 4_u64.checked_add(u64::from(index).checked_mul(4)?)?;
        read_u32(self.bytes, offset, FormatStructure::OrdinalVector).ok()
    }
}

/// 已完成 count/row/field registry 预检的一层 `RecordVector`。
#[derive(Clone, Copy, Debug)]
pub struct RegistryCheckedRecordVectorView<'a> {
    bytes: &'a [u8],
    row_schema: &'static PortableRowSchema,
    count: u32,
}

impl<'a> RegistryCheckedRecordVectorView<'a> {
    #[must_use]
    pub const fn len(self) -> u32 {
        self.count
    }

    #[must_use]
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    /// 按 wire 顺序遍历全部已受检 nested row；每行只推进一次游标。
    #[must_use]
    pub fn rows(self) -> RegistryCheckedRowIter<'a> {
        RegistryCheckedRowIter {
            table: None,
            bytes: self.bytes,
            schema: self.row_schema,
            cursor: 4,
            remaining: self.len(),
            chunk_ordinal: 0,
            chunk_remaining: self.len(),
        }
    }

    /// 按序号取得一行。
    ///
    /// 本随机访问兼容 API 从 vector 起点扫描，复杂度为 O(`ordinal`)；顺序消费应使用
    /// [`Self::rows`]。
    #[must_use]
    pub fn row(self, ordinal: u32) -> Option<RegistryCheckedRowView<'a>> {
        if ordinal >= self.len() {
            return None;
        }
        let mut cursor = 4;
        for current in 0..=ordinal {
            let row_byte_length = read_u64(self.bytes, cursor, FormatStructure::Row).ok()?;
            if current == ordinal {
                let bytes =
                    checked_slice(self.bytes, cursor, row_byte_length, FormatStructure::Row)
                        .ok()?;
                let field_count = read_u32(bytes, 8, FormatStructure::Row).ok()?;
                return Some(RegistryCheckedRowView {
                    bytes,
                    schema: self.row_schema,
                    field_count,
                });
            }
            cursor = cursor.checked_add(row_byte_length)?;
        }
        None
    }
}

/// [`RegistryCheckedObjectView`] 的 wire-order section 迭代器。
#[derive(Clone, Debug)]
pub struct RegistryCheckedSectionIter<'a> {
    object: RegistryCheckedObjectView<'a>,
    ordinal: u32,
}

impl<'a> Iterator for RegistryCheckedSectionIter<'a> {
    type Item = RegistryCheckedSectionView<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let section = self.object.section(self.ordinal)?;
        self.ordinal = self.ordinal.checked_add(1)?;
        Some(section)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        remaining_size_hint(self.object.section_count().saturating_sub(self.ordinal))
    }
}

impl core::iter::FusedIterator for RegistryCheckedSectionIter<'_> {}

/// [`RegistryCheckedSectionView`] 的单游标 wire-order table 迭代器。
#[derive(Clone, Debug)]
pub struct RegistryCheckedTableIter<'a> {
    section: RegistryCheckedSectionView<'a>,
    ordinal: u32,
}

impl<'a> Iterator for RegistryCheckedTableIter<'a> {
    type Item = RegistryCheckedTableView<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let table = self.section.table(self.ordinal)?;
        self.ordinal = self.ordinal.checked_add(1)?;
        Some(table)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        remaining_size_hint(self.section.table_count().saturating_sub(self.ordinal))
    }
}

impl core::iter::FusedIterator for RegistryCheckedTableIter<'_> {}

/// table 或一层 `RecordVector` 的单游标 wire-order row 迭代器。
#[derive(Clone, Debug)]
pub struct RegistryCheckedRowIter<'a> {
    table: Option<RegistryCheckedTableView<'a>>,
    bytes: &'a [u8],
    schema: &'static PortableRowSchema,
    cursor: u64,
    remaining: u32,
    chunk_ordinal: u32,
    chunk_remaining: u32,
}

impl<'a> RegistryCheckedRowIter<'a> {
    fn for_table(table: RegistryCheckedTableView<'a>) -> Self {
        let bytes = table.chunk_bytes(0).unwrap_or(&table.section_bytes[..0]);
        let chunk_remaining = if table.chunk_count == 0 {
            0
        } else {
            read_u32(bytes, 4, FormatStructure::Table).unwrap_or(0)
        };
        Self {
            table: Some(table),
            bytes,
            schema: table.schema.row,
            cursor: TABLE_HEADER_BYTES,
            remaining: table.row_count,
            chunk_ordinal: 0,
            chunk_remaining,
        }
    }
}

impl<'a> Iterator for RegistryCheckedRowIter<'a> {
    type Item = RegistryCheckedRowView<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        if self.chunk_remaining == 0
            && let Some(table) = self.table
        {
            self.chunk_ordinal = self.chunk_ordinal.checked_add(1)?;
            self.bytes = table.chunk_bytes(self.chunk_ordinal)?;
            self.cursor = TABLE_HEADER_BYTES;
            self.chunk_remaining = read_u32(self.bytes, 4, FormatStructure::Table).ok()?;
        }
        let Some(row_byte_length) = read_u64(self.bytes, self.cursor, FormatStructure::Row).ok()
        else {
            self.remaining = 0;
            return None;
        };
        let Some(bytes) = checked_slice(
            self.bytes,
            self.cursor,
            row_byte_length,
            FormatStructure::Row,
        )
        .ok() else {
            self.remaining = 0;
            return None;
        };
        let Some(field_count) = read_u32(bytes, 8, FormatStructure::Row).ok() else {
            self.remaining = 0;
            return None;
        };
        let Some(next_cursor) = self.cursor.checked_add(row_byte_length) else {
            self.remaining = 0;
            return None;
        };
        self.cursor = next_cursor;
        self.remaining -= 1;
        self.chunk_remaining -= 1;
        Some(RegistryCheckedRowView {
            bytes,
            schema: self.schema,
            field_count,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        remaining_size_hint(self.remaining)
    }
}

impl core::iter::FusedIterator for RegistryCheckedRowIter<'_> {}

/// [`RegistryCheckedRowView`] 的单游标 wire-order field 迭代器。
#[derive(Clone, Debug)]
pub struct RegistryCheckedFieldIter<'a> {
    bytes: &'a [u8],
    schemas: &'static [PortableFieldSchema],
    schema_index: usize,
    cursor: u64,
    remaining: u32,
}

impl<'a> Iterator for RegistryCheckedFieldIter<'a> {
    type Item = RegistryCheckedFieldView<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let Some(tag) = read_u16(self.bytes, self.cursor, FormatStructure::Field).ok() else {
            self.remaining = 0;
            return None;
        };
        while self
            .schemas
            .get(self.schema_index)
            .is_some_and(|field| field.tag < tag)
        {
            self.schema_index += 1;
        }
        let Some(schema) = self.schemas.get(self.schema_index) else {
            self.remaining = 0;
            return None;
        };
        if schema.tag != tag {
            self.remaining = 0;
            return None;
        }
        let Some(value_byte_length) =
            read_u64(self.bytes, self.cursor + 4, FormatStructure::Field).ok()
        else {
            self.remaining = 0;
            return None;
        };
        let Some(field_byte_length) = FIELD_HEADER_BYTES.checked_add(value_byte_length) else {
            self.remaining = 0;
            return None;
        };
        let Some(bytes) = checked_slice(
            self.bytes,
            self.cursor,
            field_byte_length,
            FormatStructure::Field,
        )
        .ok() else {
            self.remaining = 0;
            return None;
        };
        let Some(value) = checked_slice(
            bytes,
            FIELD_HEADER_BYTES,
            value_byte_length,
            FormatStructure::FieldValue,
        )
        .ok() else {
            self.remaining = 0;
            return None;
        };
        let Some(next_cursor) = self.cursor.checked_add(field_byte_length) else {
            self.remaining = 0;
            return None;
        };
        self.schema_index += 1;
        self.cursor = next_cursor;
        self.remaining -= 1;
        Some(RegistryCheckedFieldView { value, schema })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        remaining_size_hint(self.remaining)
    }
}

impl core::iter::FusedIterator for RegistryCheckedFieldIter<'_> {}

fn remaining_size_hint(remaining: u32) -> (usize, Option<usize>) {
    match usize::try_from(remaining) {
        Ok(remaining) => (remaining, Some(remaining)),
        Err(_) => (usize::MAX, None),
    }
}

fn read_array_value<const N: usize>(value: &[u8]) -> Result<[u8; N], FormatError> {
    value.try_into().map_err(|_| FormatError::LengthMismatch {
        structure: FormatStructure::FieldValue,
        declared: value.len() as u64,
        actual: N as u64,
    })
}

/// 对完整对象执行前导、目录与静态 registry 的 fail-closed 结构预检。
pub fn preflight_object_registry(
    bytes: &[u8],
    expected_kind: PortableObjectKind,
    limits: FormatLimits,
) -> Result<RegistryCheckedObjectView<'_>, FormatError> {
    preflight_object_registry_at(
        bytes,
        expected_kind,
        expected_kind.format_version(),
        portable_object_schema(expected_kind),
        limits,
    )
}

fn preflight_object_registry_at<'a>(
    bytes: &'a [u8],
    expected_kind: PortableObjectKind,
    expected_format_version: u16,
    schema: &'static PortableObjectSchema,
    limits: FormatLimits,
) -> Result<RegistryCheckedObjectView<'a>, FormatError> {
    let framing = crate::framing::preflight_object_framing_at(
        bytes,
        expected_kind,
        expected_format_version,
        limits,
    )?;

    for (ordinal, section_schema) in schema.sections.iter().enumerate() {
        let ordinal = u32::try_from(ordinal).map_err(|_| FormatError::ArithmeticOverflow {
            structure: FormatStructure::SectionDirectory,
        })?;
        let section = framing
            .section(ordinal)
            .ok_or(FormatError::BindingMismatch {
                structure: FormatStructure::Section,
            })?;
        if expected_kind == PortableObjectKind::CanonicalPublicationDescriptor {
            preflight_singleton_section(section.bytes(), section_schema, limits)?;
        } else {
            preflight_chunked_section(section.bytes(), expected_kind, section_schema, limits)?;
        }
    }

    Ok(RegistryCheckedObjectView {
        framing,
        limits,
        schema,
    })
}

fn preflight_singleton_section(
    bytes: &[u8],
    schema: &'static PortableSectionSchema,
    limits: FormatLimits,
) -> Result<(), FormatError> {
    let [table_schema] = schema.tables else {
        return Err(FormatError::BindingMismatch {
            structure: FormatStructure::Section,
        });
    };
    let mut budget = PreflightBudget::default();
    preflight_table_with_registry(bytes, table_schema, limits, &mut budget)?;
    Ok(())
}

fn preflight_chunked_section(
    bytes: &[u8],
    object_kind: PortableObjectKind,
    schema: &'static PortableSectionSchema,
    limits: FormatLimits,
) -> Result<(), FormatError> {
    let chunk_count = read_u32(bytes, 0, FormatStructure::Section)?;
    if chunk_count > limits.max_chunks_per_section() {
        return Err(FormatError::LimitExceeded {
            dimension: LimitDimension::ChunksPerSection,
            actual: u64::from(chunk_count),
            limit: u64::from(limits.max_chunks_per_section()),
        });
    }
    let entry_byte_length = read_u16(bytes, 4, FormatStructure::Section)?;
    if u64::from(entry_byte_length) != TABLE_CHUNK_DIRECTORY_ENTRY_BYTE_LENGTH {
        return Err(FormatError::NonCanonicalValue {
            structure: FormatStructure::Section,
            offset: 4,
        });
    }
    if read_u16(bytes, 6, FormatStructure::Section)? != 0 {
        return Err(FormatError::NonCanonicalValue {
            structure: FormatStructure::Section,
            offset: 6,
        });
    }
    let directory_byte_length = read_u64(bytes, 8, FormatStructure::Section)?;
    let expected_directory_byte_length = CHUNKED_SECTION_PREAMBLE_BYTE_LENGTH
        .checked_add(
            u64::from(chunk_count)
                .checked_mul(TABLE_CHUNK_DIRECTORY_ENTRY_BYTE_LENGTH)
                .ok_or(FormatError::ArithmeticOverflow {
                    structure: FormatStructure::ChunkDirectory,
                })?,
        )
        .ok_or(FormatError::ArithmeticOverflow {
            structure: FormatStructure::ChunkDirectory,
        })?;
    if directory_byte_length != expected_directory_byte_length {
        return Err(FormatError::LengthMismatch {
            structure: FormatStructure::ChunkDirectory,
            declared: directory_byte_length,
            actual: expected_directory_byte_length,
        });
    }
    let directory = checked_slice(
        bytes,
        CHUNKED_SECTION_PREAMBLE_BYTE_LENGTH,
        expected_directory_byte_length - CHUNKED_SECTION_PREAMBLE_BYTE_LENGTH,
        FormatStructure::ChunkDirectory,
    )?;

    let mut expected_offset = directory_byte_length;
    let mut schema_index = 0_usize;
    let mut previous_table_kind = 0_u16;
    let mut expected_chunk_index = 0_u32;
    let mut expected_first_row = 0_u32;
    let mut previous_canonical_chunk = None;
    for entry_index in 0..chunk_count {
        let entry = u64::from(entry_index) * TABLE_CHUNK_DIRECTORY_ENTRY_BYTE_LENGTH;
        let table_kind = read_u16(directory, entry, FormatStructure::ChunkDirectoryEntry)?;
        while schema
            .tables
            .get(schema_index)
            .is_some_and(|table| table.kind < table_kind)
        {
            validate_logical_cardinality(schema.tables[schema_index], 0)?;
            schema_index += 1;
        }
        let table_schema = schema
            .tables
            .get(schema_index)
            .ok_or(FormatError::UnknownKind {
                structure: FormatStructure::ChunkDirectoryEntry,
                code: u64::from(table_kind),
            })?;
        if table_schema.kind != table_kind {
            return Err(FormatError::UnknownKind {
                structure: FormatStructure::ChunkDirectoryEntry,
                code: u64::from(table_kind),
            });
        }

        if table_kind != previous_table_kind {
            previous_table_kind = table_kind;
            expected_chunk_index = 0;
            expected_first_row = 0;
            previous_canonical_chunk = None;
        }
        let table_schema_version =
            read_u16(directory, entry + 2, FormatStructure::ChunkDirectoryEntry)?;
        if table_schema_version != 1 {
            return Err(FormatError::UnsupportedVersion {
                structure: FormatStructure::ChunkDirectoryEntry,
                actual: u64::from(table_schema_version),
                expected: 1,
            });
        }
        let chunk_index = read_u32(directory, entry + 4, FormatStructure::ChunkDirectoryEntry)?;
        if chunk_index != expected_chunk_index {
            return Err(FormatError::NonCanonicalOrder {
                structure: FormatStructure::ChunkDirectory,
                previous: u64::from(expected_chunk_index),
                current: u64::from(chunk_index),
            });
        }
        let first_row = read_u32(directory, entry + 8, FormatStructure::ChunkDirectoryEntry)?;
        if first_row != expected_first_row {
            return Err(FormatError::GapOrOverlap {
                expected_offset: u64::from(expected_first_row),
                actual_offset: u64::from(first_row),
            });
        }
        let row_count = read_u32(directory, entry + 12, FormatStructure::ChunkDirectoryEntry)?;
        if row_count == 0 {
            return Err(FormatError::NonCanonicalValue {
                structure: FormatStructure::ChunkDirectoryEntry,
                offset: entry + 12,
            });
        }
        check_source_location_rows(object_kind, schema.kind, table_kind, row_count, limits)?;
        if read_u32(directory, entry + 16, FormatStructure::ChunkDirectoryEntry)? != 0
            || read_u32(directory, entry + 20, FormatStructure::ChunkDirectoryEntry)? != 0
        {
            return Err(FormatError::NonCanonicalValue {
                structure: FormatStructure::ChunkDirectoryEntry,
                offset: entry + 16,
            });
        }
        let byte_offset = read_u64(directory, entry + 24, FormatStructure::ChunkDirectoryEntry)?;
        if byte_offset != expected_offset {
            return Err(FormatError::GapOrOverlap {
                expected_offset,
                actual_offset: byte_offset,
            });
        }
        let byte_length = read_u64(directory, entry + 32, FormatStructure::ChunkDirectoryEntry)?;
        if byte_length > limits.config().max_table_chunk_bytes {
            return Err(FormatError::LimitExceeded {
                dimension: LimitDimension::TableChunkBytes,
                actual: byte_length,
                limit: limits.config().max_table_chunk_bytes,
            });
        }
        let chunk = checked_slice(bytes, byte_offset, byte_length, FormatStructure::Table)?;
        let expected_digest: [u8; 32] = checked_slice(
            directory,
            entry + 40,
            32,
            FormatStructure::ChunkDirectoryEntry,
        )?
        .try_into()
        .map_err(|_| FormatError::BindingMismatch {
            structure: FormatStructure::ChunkDirectoryEntry,
        })?;
        let actual_digest: [u8; 32] = Sha256::digest(chunk).into();
        if actual_digest != expected_digest {
            return Err(FormatError::DigestMismatch {
                structure: FormatStructure::ChunkDirectoryEntry,
            });
        }
        if read_u16(chunk, 0, FormatStructure::Table)? != table_kind
            || read_u16(chunk, 2, FormatStructure::Table)? != table_schema_version
            || read_u32(chunk, 4, FormatStructure::Table)? != row_count
        {
            return Err(FormatError::BindingMismatch {
                structure: FormatStructure::ChunkDirectoryEntry,
            });
        }
        let declared_chunk_length = TABLE_HEADER_BYTES
            .checked_add(read_u64(chunk, 8, FormatStructure::Table)?)
            .ok_or(FormatError::ArithmeticOverflow {
                structure: FormatStructure::Table,
            })?;
        if declared_chunk_length != byte_length {
            return Err(FormatError::LengthMismatch {
                structure: FormatStructure::ChunkDirectoryEntry,
                declared: byte_length,
                actual: declared_chunk_length,
            });
        }
        let mut budget = PreflightBudget::default();
        let summary = preflight_table_with_registry(chunk, table_schema, limits, &mut budget)?;
        if let Some(previous) = previous_canonical_chunk
            && canonical_chunk_with_appended_row(
                object_kind,
                schema.kind,
                table_kind,
                previous,
                summary.first_row_metrics(),
            )
            .is_some()
        {
            return Err(FormatError::NonCanonicalValue {
                structure: FormatStructure::ChunkDirectoryEntry,
                offset: entry + 12,
            });
        }
        previous_canonical_chunk = Some(CanonicalChunkMetrics {
            row_count,
            exact_byte_length: byte_length,
            total_utf8_bytes: summary.total_utf8_bytes(),
            total_vector_bytes: summary.total_vector_bytes(),
        });

        expected_first_row =
            expected_first_row
                .checked_add(row_count)
                .ok_or(FormatError::ArithmeticOverflow {
                    structure: FormatStructure::TableRows,
                })?;
        expected_chunk_index =
            expected_chunk_index
                .checked_add(1)
                .ok_or(FormatError::ArithmeticOverflow {
                    structure: FormatStructure::ChunkDirectory,
                })?;
        expected_offset =
            expected_offset
                .checked_add(byte_length)
                .ok_or(FormatError::ArithmeticOverflow {
                    structure: FormatStructure::Section,
                })?;

        let next_kind = if entry_index + 1 < chunk_count {
            read_u16(
                directory,
                entry + TABLE_CHUNK_DIRECTORY_ENTRY_BYTE_LENGTH,
                FormatStructure::ChunkDirectoryEntry,
            )?
        } else {
            0
        };
        if next_kind != table_kind {
            validate_logical_cardinality(*table_schema, expected_first_row)?;
            schema_index += 1;
        }
    }
    while let Some(table_schema) = schema.tables.get(schema_index) {
        validate_logical_cardinality(*table_schema, 0)?;
        schema_index += 1;
    }
    if expected_offset != bytes.len() as u64 {
        return Err(FormatError::LengthMismatch {
            structure: FormatStructure::Section,
            declared: expected_offset,
            actual: bytes.len() as u64,
        });
    }
    Ok(())
}

fn validate_logical_cardinality(
    schema: PortableTableSchema,
    row_count: u32,
) -> Result<(), FormatError> {
    let valid = match schema.cardinality {
        PortableRowCardinality::Any => true,
        PortableRowCardinality::AtMostOne => row_count <= 1,
        PortableRowCardinality::ExactlyOne => row_count == 1,
    };
    if valid {
        Ok(())
    } else {
        Err(FormatError::BindingMismatch {
            structure: FormatStructure::TableRows,
        })
    }
}

fn check_source_location_rows(
    object_kind: PortableObjectKind,
    section_kind: u16,
    table_kind: u16,
    row_count: u32,
    limits: FormatLimits,
) -> Result<(), FormatError> {
    if object_kind != PortableObjectKind::SourceMap || section_kind != 2 || table_kind != 3 {
        return Ok(());
    }
    let limit = limits.max_source_location_rows_per_chunk();
    if row_count > limit {
        return Err(FormatError::LimitExceeded {
            dimension: LimitDimension::SourceLocationRowsPerChunk,
            actual: u64::from(row_count),
            limit: u64::from(limit),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::vec;
    use std::vec::Vec;

    use laneflow_static_contract::{
        CHUNKED_SECTION_PREAMBLE_BYTE_LENGTH, FORMAT_HARD_MAX_SOURCE_LOCATION_ROWS_PER_CHUNK,
        OBJECT_PREAMBLE_BYTE_LENGTH, PortableFieldSchema, PortableFieldType,
        PortableRowCardinality, PortableRowSchema, PortableRowShape,
        SECTION_DIRECTORY_ENTRY_BYTE_LENGTH, TABLE_CHUNK_DIRECTORY_ENTRY_BYTE_LENGTH,
    };
    use sha2::{Digest, Sha256};

    use super::*;
    use crate::{
        FormatErrorClass, FormatLimitConfig, LimitDimension, table::preflight_table_with_registry,
    };

    fn default_value(field: &PortableFieldSchema) -> Vec<u8> {
        match field.field_type {
            PortableFieldType::U8 => vec![0],
            PortableFieldType::U16 => vec![0; 2],
            PortableFieldType::U32 | PortableFieldType::F32 | PortableFieldType::I32 => vec![0; 4],
            PortableFieldType::U64 | PortableFieldType::F64 => vec![0; 8],
            PortableFieldType::StableId128 => vec![0; 16],
            PortableFieldType::Sha256 => vec![0; 32],
            PortableFieldType::Utf8 | PortableFieldType::Bytes => Vec::new(),
            PortableFieldType::OrdinalVectorU32 | PortableFieldType::RecordVector => {
                0_u32.to_le_bytes().to_vec()
            }
        }
    }

    fn encoded_field_with_value(
        field: &PortableFieldSchema,
        field_type: PortableFieldType,
        value: &[u8],
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&field.tag.to_le_bytes());
        bytes.push(field_type as u8);
        bytes.push(0);
        bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
        bytes.extend_from_slice(value);
        bytes
    }

    fn encoded_field(field: &PortableFieldSchema, field_type: PortableFieldType) -> Vec<u8> {
        encoded_field_with_value(field, field_type, &default_value(field))
    }

    fn encoded_row(schema: &PortableRowSchema, omit_tag: Option<u16>) -> Vec<u8> {
        let fields = schema
            .fields
            .iter()
            .filter(|field| {
                Some(field.tag) != omit_tag
                    && field.presence == laneflow_static_contract::PortableFieldPresence::Required
            })
            .map(|field| encoded_field(field, field.field_type))
            .collect::<Vec<_>>();
        let length = 16 + fields.iter().map(Vec::len).sum::<usize>();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(length as u64).to_le_bytes());
        bytes.extend_from_slice(&(fields.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        for field in fields {
            bytes.extend_from_slice(&field);
        }
        bytes
    }

    fn encoded_variant_row(
        schema: &PortableRowSchema,
        tags: &[u16],
        discriminant_tag: u16,
        discriminant: u8,
    ) -> Vec<u8> {
        let fields = tags
            .iter()
            .map(|tag| {
                let field = schema
                    .fields
                    .iter()
                    .find(|field| field.tag == *tag)
                    .unwrap();
                if *tag == discriminant_tag {
                    encoded_field_with_value(field, field.field_type, &[discriminant])
                } else {
                    encoded_field(field, field.field_type)
                }
            })
            .collect::<Vec<_>>();
        let length = 16 + fields.iter().map(Vec::len).sum::<usize>();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(length as u64).to_le_bytes());
        bytes.extend_from_slice(&(fields.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        for field in fields {
            bytes.extend_from_slice(&field);
        }
        bytes
    }

    fn table_with_rows(
        schema: &laneflow_static_contract::PortableTableSchema,
        rows: &[Vec<u8>],
    ) -> Vec<u8> {
        let rows_length = rows.iter().map(Vec::len).sum::<usize>();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&schema.kind.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(rows_length as u64).to_le_bytes());
        for row in rows {
            bytes.extend_from_slice(row);
        }
        bytes
    }

    fn table_with_repeated_row(
        schema: &laneflow_static_contract::PortableTableSchema,
        row: &[u8],
        row_count: u32,
    ) -> Vec<u8> {
        let rows_length = u64::from(row_count) * row.len() as u64;
        let mut bytes = Vec::with_capacity(
            usize::try_from(TABLE_HEADER_BYTES + rows_length).expect("test table length"),
        );
        bytes.extend_from_slice(&schema.kind.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&row_count.to_le_bytes());
        bytes.extend_from_slice(&rows_length.to_le_bytes());
        for _ in 0..row_count {
            bytes.extend_from_slice(row);
        }
        bytes
    }

    fn encoded_table(schema: &laneflow_static_contract::PortableTableSchema) -> Vec<u8> {
        let rows = if schema.cardinality == PortableRowCardinality::ExactlyOne {
            vec![encoded_row(schema.row, None)]
        } else {
            Vec::new()
        };
        table_with_rows(schema, &rows)
    }

    fn encoded_section(kind: PortableObjectKind, tables: Vec<Vec<u8>>) -> Vec<u8> {
        if kind == PortableObjectKind::CanonicalPublicationDescriptor {
            assert_eq!(tables.len(), 1);
            return tables.into_iter().next().unwrap();
        }

        let tables = tables
            .into_iter()
            .filter(|table| u32::from_le_bytes(table[4..8].try_into().unwrap()) != 0)
            .collect::<Vec<_>>();
        let chunk_count = u32::try_from(tables.len()).unwrap();
        let directory_length = CHUNKED_SECTION_PREAMBLE_BYTE_LENGTH
            + u64::from(chunk_count) * TABLE_CHUNK_DIRECTORY_ENTRY_BYTE_LENGTH;
        let mut bytes = vec![0_u8; usize::try_from(directory_length).unwrap()];
        bytes[0..4].copy_from_slice(&chunk_count.to_le_bytes());
        bytes[4..6]
            .copy_from_slice(&(TABLE_CHUNK_DIRECTORY_ENTRY_BYTE_LENGTH as u16).to_le_bytes());
        bytes[8..16].copy_from_slice(&directory_length.to_le_bytes());
        let mut chunk_offset = directory_length;
        let mut previous_table_kind = 0_u16;
        let mut chunk_index = 0_u32;
        let mut first_row = 0_u32;
        for (ordinal, table) in tables.into_iter().enumerate() {
            let entry = CHUNKED_SECTION_PREAMBLE_BYTE_LENGTH as usize
                + ordinal * TABLE_CHUNK_DIRECTORY_ENTRY_BYTE_LENGTH as usize;
            let table_kind = u16::from_le_bytes(table[0..2].try_into().unwrap());
            let row_count = u32::from_le_bytes(table[4..8].try_into().unwrap());
            if table_kind != previous_table_kind {
                previous_table_kind = table_kind;
                chunk_index = 0;
                first_row = 0;
            }
            bytes[entry..entry + 2].copy_from_slice(&table_kind.to_le_bytes());
            bytes[entry + 2..entry + 4].copy_from_slice(&1_u16.to_le_bytes());
            bytes[entry + 4..entry + 8].copy_from_slice(&chunk_index.to_le_bytes());
            bytes[entry + 8..entry + 12].copy_from_slice(&first_row.to_le_bytes());
            bytes[entry + 12..entry + 16].copy_from_slice(&row_count.to_le_bytes());
            bytes[entry + 24..entry + 32].copy_from_slice(&chunk_offset.to_le_bytes());
            bytes[entry + 32..entry + 40].copy_from_slice(&(table.len() as u64).to_le_bytes());
            bytes[entry + 40..entry + 72].copy_from_slice(&Sha256::digest(&table));
            chunk_index += 1;
            first_row += row_count;
            chunk_offset += table.len() as u64;
            bytes.extend_from_slice(&table);
        }
        bytes
    }

    fn encoded_object(kind: PortableObjectKind) -> Vec<u8> {
        let schema = portable_object_schema(kind);
        let sections = schema
            .sections
            .iter()
            .map(|section| {
                encoded_section(kind, section.tables.iter().map(encoded_table).collect())
            })
            .collect::<Vec<_>>();
        object_from_sections(kind, &sections)
    }

    fn object_from_sections(kind: PortableObjectKind, sections: &[Vec<u8>]) -> Vec<u8> {
        assert_eq!(sections.len(), kind.section_count() as usize);
        let total = sections
            .iter()
            .map(Vec::len)
            .try_fold(kind.first_section_offset(), |total, length| {
                total.checked_add(length as u64)
            })
            .unwrap();
        let mut bytes = vec![0_u8; kind.first_section_offset() as usize];
        bytes[0..4].copy_from_slice(&kind.magic());
        bytes[4..6].copy_from_slice(&kind.format_version().to_le_bytes());
        bytes[6..8].copy_from_slice(&OBJECT_PREAMBLE_BYTE_LENGTH.to_le_bytes());
        bytes[12..16].copy_from_slice(&kind.section_count().to_le_bytes());
        bytes[16..24].copy_from_slice(&u64::from(OBJECT_PREAMBLE_BYTE_LENGTH).to_le_bytes());
        bytes[24..32].copy_from_slice(&total.to_le_bytes());
        let mut section_offset = kind.first_section_offset();
        for (ordinal, section) in sections.iter().enumerate() {
            let entry = usize::from(OBJECT_PREAMBLE_BYTE_LENGTH)
                + ordinal * SECTION_DIRECTORY_ENTRY_BYTE_LENGTH as usize;
            bytes[entry..entry + 2]
                .copy_from_slice(&u16::try_from(ordinal + 1).unwrap().to_le_bytes());
            bytes[entry + 2..entry + 4]
                .copy_from_slice(&kind.section_format_version().to_le_bytes());
            bytes[entry + 8..entry + 16].copy_from_slice(&section_offset.to_le_bytes());
            bytes[entry + 16..entry + 24].copy_from_slice(&(section.len() as u64).to_le_bytes());
            section_offset += section.len() as u64;
            bytes.extend_from_slice(section);
        }
        bytes
    }

    #[test]
    fn every_object_registry_accepts_a_minimal_structural_traversal_fixture() {
        // 这些 bytes 由 registry 生成，只证明全部登记分支可遍历；它不是独立 known vector。
        for kind in PortableObjectKind::ALL {
            let bytes = encoded_object(kind);
            let checked = preflight_object_registry(&bytes, kind, FormatLimits::HARD)
                .unwrap_or_else(|error| {
                    panic!("{kind:?} rejected generated registry fixture: {error:?}")
                });
            assert_eq!(checked.kind(), kind);
            assert_eq!(checked.bytes(), bytes);
            assert_eq!(checked.section_count(), kind.section_count());
            assert_eq!(checked.section(0).unwrap().kind(), 1);
        }
    }

    #[test]
    fn every_object_byte_boundary_truncates_before_a_checked_view_exists() {
        for kind in PortableObjectKind::ALL {
            let original = encoded_object(kind);
            for boundary in 0..original.len() {
                let mut truncated = original[..boundary].to_vec();
                if boundary >= usize::from(OBJECT_PREAMBLE_BYTE_LENGTH) {
                    truncated[24..32]
                        .copy_from_slice(&u64::try_from(boundary).unwrap().to_le_bytes());
                }
                let class = preflight_object_registry(&truncated, kind, FormatLimits::HARD)
                    .unwrap_err()
                    .class();
                assert!(
                    matches!(
                        class,
                        FormatErrorClass::Truncated | FormatErrorClass::LengthMismatch
                    ),
                    "{kind:?} boundary {boundary} returned {class:?}"
                );
            }
        }
    }

    #[test]
    fn one_bit_corruption_in_every_object_section_fails_registry_preflight() {
        for kind in PortableObjectKind::ALL {
            let original = encoded_object(kind);
            let framing =
                crate::preflight_object_framing(&original, kind, FormatLimits::HARD).unwrap();
            for ordinal in 0..framing.section_count() {
                let section = framing.section(ordinal).unwrap();
                let offset = section.bytes().as_ptr() as usize - original.as_ptr() as usize;
                let mut corrupted = original.clone();
                corrupted[offset] ^= 1;
                assert!(
                    preflight_object_registry(&corrupted, kind, FormatLimits::HARD).is_err(),
                    "{kind:?} section {} accepted a one-bit table-count corruption",
                    ordinal + 1
                );
            }
        }
    }

    #[test]
    fn every_object_rejects_section_count_minus_and_plus_one() {
        for kind in PortableObjectKind::ALL {
            let original = encoded_object(kind);
            for count in [kind.section_count() - 1, kind.section_count() + 1] {
                let mut bytes = original.clone();
                bytes[12..16].copy_from_slice(&count.to_le_bytes());
                assert_eq!(
                    preflight_object_registry(&bytes, kind, FormatLimits::HARD)
                        .unwrap_err()
                        .class(),
                    FormatErrorClass::LengthMismatch,
                    "{kind:?} accepted sectionCount={count}"
                );
            }
        }
    }

    #[test]
    fn every_chunked_section_rejects_a_wrong_chunk_count_before_chunk_walk() {
        for kind in PortableObjectKind::ALL {
            if kind == PortableObjectKind::CanonicalPublicationDescriptor {
                continue;
            }
            let original = encoded_object(kind);
            for (section_ordinal, section_schema) in
                portable_object_schema(kind).sections.iter().enumerate()
            {
                let entry = usize::from(OBJECT_PREAMBLE_BYTE_LENGTH)
                    + section_ordinal
                        * usize::try_from(SECTION_DIRECTORY_ENTRY_BYTE_LENGTH).unwrap();
                let section_offset =
                    u64::from_le_bytes(original[entry + 8..entry + 16].try_into().unwrap())
                        as usize;
                let exact = u32::from_le_bytes(
                    original[section_offset..section_offset + 4]
                        .try_into()
                        .unwrap(),
                );
                let mut wrong_counts = vec![exact + 1];
                if exact != 0 {
                    wrong_counts.push(exact - 1);
                }
                for count in wrong_counts {
                    let mut bytes = original.clone();
                    bytes[section_offset..section_offset + 4].copy_from_slice(&count.to_le_bytes());
                    assert!(
                        preflight_object_registry(&bytes, kind, FormatLimits::HARD).is_err(),
                        "{kind:?} section {} accepted chunkCount={count}",
                        section_schema.kind,
                    );
                }
            }
        }
    }

    #[test]
    fn chunked_reader_rejects_an_early_split_that_the_hard_budget_does_not_require() {
        let kind = PortableObjectKind::CanonicalArtifact;
        let schema = portable_object_schema(kind);
        let table_schema = &schema.sections[2].tables[0];
        let row = encoded_row(table_schema.row, None);
        let early_split = encoded_section(
            kind,
            vec![
                table_with_rows(table_schema, core::slice::from_ref(&row)),
                table_with_rows(table_schema, core::slice::from_ref(&row)),
            ],
        );
        let sections = schema
            .sections
            .iter()
            .enumerate()
            .map(|(ordinal, section)| {
                if ordinal == 2 {
                    early_split.clone()
                } else {
                    encoded_section(kind, section.tables.iter().map(encoded_table).collect())
                }
            })
            .collect::<Vec<_>>();
        let bytes = object_from_sections(kind, &sections);

        assert_eq!(
            preflight_object_registry(&bytes, kind, FormatLimits::HARD)
                .unwrap_err()
                .class(),
            FormatErrorClass::NonCanonicalValue
        );
    }

    #[test]
    fn every_uniform_row_registry_rejects_each_missing_required_and_unknown_field() {
        for kind in PortableObjectKind::ALL {
            for section in portable_object_schema(kind).sections {
                for table_schema in section.tables {
                    if table_schema.row.shape != PortableRowShape::Uniform {
                        continue;
                    }

                    let valid = encoded_row(table_schema.row, None);
                    let table = table_with_rows(table_schema, core::slice::from_ref(&valid));
                    preflight_table_with_registry(
                        &table,
                        table_schema,
                        FormatLimits::HARD,
                        &mut PreflightBudget::default(),
                    )
                    .unwrap_or_else(|error| {
                        panic!(
                            "{kind:?} section {} table {} rejected its exact uniform row: {error:?}",
                            section.kind, table_schema.kind
                        )
                    });

                    for required in table_schema.row.fields.iter().filter(|field| {
                        field.presence == laneflow_static_contract::PortableFieldPresence::Required
                    }) {
                        let missing = encoded_row(table_schema.row, Some(required.tag));
                        let table = table_with_rows(table_schema, &[missing]);
                        assert_eq!(
                            preflight_table_with_registry(
                                &table,
                                table_schema,
                                FormatLimits::HARD,
                                &mut PreflightBudget::default(),
                            )
                            .unwrap_err()
                            .class(),
                            FormatErrorClass::BindingMismatch,
                            "{kind:?} section {} table {} accepted missing tag {}",
                            section.kind,
                            table_schema.kind,
                            required.tag
                        );
                    }

                    let last_field_offset = valid.len()
                        - table_schema
                            .row
                            .fields
                            .iter()
                            .rfind(|field| {
                                field.presence
                                    == laneflow_static_contract::PortableFieldPresence::Required
                            })
                            .map_or(0, |field| 12 + default_value(field).len());
                    let mut unknown = valid;
                    let unknown_tag = table_schema.row.fields.last().unwrap().tag + 1;
                    unknown[last_field_offset..last_field_offset + 2]
                        .copy_from_slice(&unknown_tag.to_le_bytes());
                    let table = table_with_rows(table_schema, &[unknown]);
                    assert_eq!(
                        preflight_table_with_registry(
                            &table,
                            table_schema,
                            FormatLimits::HARD,
                            &mut PreflightBudget::default(),
                        )
                        .unwrap_err()
                        .class(),
                        FormatErrorClass::UnknownKind,
                        "{kind:?} section {} table {} accepted unknown tag {unknown_tag}",
                        section.kind,
                        table_schema.kind
                    );
                }
            }
        }
    }

    #[test]
    fn registry_checked_view_exposes_only_registered_typed_values() {
        let kind = PortableObjectKind::CanonicalArtifact;
        let bytes = encoded_object(kind);
        let checked = preflight_object_registry(&bytes, kind, FormatLimits::HARD).unwrap();
        let section = checked.section(0).unwrap();
        assert_eq!(section.table_count(), 1);
        let table = section.table(0).unwrap();
        assert_eq!(table.kind(), 1);
        assert_eq!(table.row_count(), 1);
        let row = table.row(0).unwrap();
        assert_eq!(row.field_count(), 6);
        let field = row.field_by_tag(1).unwrap();
        assert_eq!(field.field_type(), PortableFieldType::U16);
        assert!(matches!(
            field.value().unwrap(),
            RegistryCheckedFieldValue::U16(0)
        ));
        assert!(row.field_by_tag(7).is_none());
        assert!(table.row(1).is_none());
        assert!(section.table(1).is_none());

        assert_eq!(checked.sections().count(), checked.section_count() as usize);
        assert_eq!(section.tables().count(), section.table_count() as usize);
        assert_eq!(table.rows().count(), table.row_count() as usize);
        assert_eq!(
            row.fields()
                .map(RegistryCheckedFieldView::tag)
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5, 6]
        );
    }

    #[test]
    fn registry_row_iterator_stays_linear_at_the_frozen_hard_limit() {
        let schema =
            &portable_object_schema(PortableObjectKind::CanonicalArtifact).sections[2].tables[0];
        assert_eq!(schema.cardinality, PortableRowCardinality::Any);
        let encoded = encoded_row(schema.row, None);
        let row_count = FormatLimitConfig::HARD.max_rows_per_chunk;
        let table_bytes = table_with_repeated_row(schema, &encoded, row_count);
        preflight_table_with_registry(
            &table_bytes,
            schema,
            FormatLimits::HARD,
            &mut PreflightBudget::default(),
        )
        .unwrap();

        let mut directory = [0_u8; TABLE_CHUNK_DIRECTORY_ENTRY_BYTE_LENGTH as usize];
        directory[0..2].copy_from_slice(&schema.kind.to_le_bytes());
        directory[2..4].copy_from_slice(&1_u16.to_le_bytes());
        directory[12..16].copy_from_slice(&row_count.to_le_bytes());
        directory[32..40].copy_from_slice(&(table_bytes.len() as u64).to_le_bytes());
        let table = RegistryCheckedTableView {
            section_bytes: &table_bytes,
            schema,
            row_count,
            directory: &directory[..],
            first_chunk: 0,
            chunk_count: 1,
            chunked: true,
        };
        let mut observed = 0_u32;
        for row in table.rows() {
            assert_eq!(row.field_count(), schema.row.fields.len() as u32);
            observed = observed.checked_add(1).unwrap();
        }
        assert_eq!(observed, row_count);
    }

    #[test]
    fn object_preflight_rejects_section_table_count_and_trailing_bytes() {
        let kind = PortableObjectKind::CanonicalPublicationDescriptor;
        let original = encoded_object(kind);
        let first_section = kind.first_section_offset() as usize;

        let mut wrong_table_kind = original.clone();
        wrong_table_kind[first_section..first_section + 2].copy_from_slice(&2_u16.to_le_bytes());
        assert!(preflight_object_registry(&wrong_table_kind, kind, FormatLimits::HARD).is_err());

        let mut trailing = encoded_object(kind);
        trailing.push(0);
        let new_object_length = trailing.len() as u64;
        trailing[24..32].copy_from_slice(&new_object_length.to_le_bytes());
        let last_entry = usize::from(OBJECT_PREAMBLE_BYTE_LENGTH)
            + (kind.section_count() as usize - 1) * SECTION_DIRECTORY_ENTRY_BYTE_LENGTH as usize;
        let old_length = u64::from_le_bytes(
            trailing[last_entry + 16..last_entry + 24]
                .try_into()
                .unwrap(),
        );
        trailing[last_entry + 16..last_entry + 24].copy_from_slice(&(old_length + 1).to_le_bytes());
        assert_eq!(
            preflight_object_registry(&trailing, kind, FormatLimits::HARD)
                .unwrap_err()
                .class(),
            FormatErrorClass::LengthMismatch
        );

        let lfca_kind = PortableObjectKind::CanonicalArtifact;
        let mut wrong_order = encoded_object(lfca_kind);
        let contract_section_entry = usize::from(OBJECT_PREAMBLE_BYTE_LENGTH);
        let contract_section_offset = u64::from_le_bytes(
            wrong_order[contract_section_entry + 8..contract_section_entry + 16]
                .try_into()
                .unwrap(),
        ) as usize;
        wrong_order[contract_section_offset + 16..contract_section_offset + 18]
            .copy_from_slice(&2_u16.to_le_bytes());
        assert!(preflight_object_registry(&wrong_order, lfca_kind, FormatLimits::HARD).is_err());

        let mut reserved_kind = encoded_object(lfca_kind);
        let reserved_section_offset = u64::from_le_bytes(
            reserved_kind[contract_section_entry + 8..contract_section_entry + 16]
                .try_into()
                .unwrap(),
        ) as usize;
        reserved_kind[reserved_section_offset + 16..reserved_section_offset + 18]
            .copy_from_slice(&99_u16.to_le_bytes());
        assert!(preflight_object_registry(&reserved_kind, lfca_kind, FormatLimits::HARD).is_err());
    }

    #[test]
    fn object_preflight_checks_chunk_directory_shape_before_reading_any_chunk() {
        let kind = PortableObjectKind::CanonicalArtifact;
        let mut bytes = encoded_object(kind);
        let first_section = kind.first_section_offset() as usize;
        bytes[first_section + 4..first_section + 6].copy_from_slice(&73_u16.to_le_bytes());

        assert_eq!(
            preflight_object_registry(&bytes, kind, FormatLimits::HARD)
                .unwrap_err()
                .class(),
            FormatErrorClass::NonCanonicalValue
        );
    }

    #[test]
    fn registry_table_preflight_rejects_missing_unknown_and_wrong_type_fields() {
        let table_schema =
            &portable_object_schema(PortableObjectKind::CanonicalPublicationDescriptor).sections[0]
                .tables[0];
        let valid_row = encoded_row(table_schema.row, None);

        let empty_singleton = table_with_rows(table_schema, &[]);
        assert_eq!(
            preflight_table_with_registry(
                &empty_singleton,
                table_schema,
                FormatLimits::HARD,
                &mut PreflightBudget::default(),
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::BindingMismatch
        );

        let missing = table_with_rows(table_schema, &[encoded_row(table_schema.row, Some(3))]);
        assert_eq!(
            preflight_table_with_registry(
                &missing,
                table_schema,
                FormatLimits::HARD,
                &mut PreflightBudget::default(),
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::BindingMismatch
        );

        let mut wrong_type_row = valid_row;
        wrong_type_row[18] = PortableFieldType::U32 as u8;
        let wrong_type = table_with_rows(table_schema, &[wrong_type_row]);
        assert_eq!(
            preflight_table_with_registry(
                &wrong_type,
                table_schema,
                FormatLimits::HARD,
                &mut PreflightBudget::default(),
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::BindingMismatch
        );

        let mut unknown_row = encoded_row(table_schema.row, None);
        let unknown_field = {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&6_u16.to_le_bytes());
            bytes.push(PortableFieldType::U8 as u8);
            bytes.push(0);
            bytes.extend_from_slice(&1_u64.to_le_bytes());
            bytes.push(0);
            bytes
        };
        let new_length = unknown_row.len() + unknown_field.len();
        unknown_row[0..8].copy_from_slice(&(new_length as u64).to_le_bytes());
        unknown_row[8..12].copy_from_slice(&6_u32.to_le_bytes());
        unknown_row.extend_from_slice(&unknown_field);
        let unknown = table_with_rows(table_schema, &[unknown_row]);
        assert_eq!(
            preflight_table_with_registry(
                &unknown,
                table_schema,
                FormatLimits::HARD,
                &mut PreflightBudget::default(),
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::UnknownKind
        );

        for tags in [[1, 1], [2, 1]] {
            let noncanonical_row = encoded_variant_row(table_schema.row, &tags, 0, 0);
            let noncanonical = table_with_rows(table_schema, &[noncanonical_row]);
            assert_eq!(
                preflight_table_with_registry(
                    &noncanonical,
                    table_schema,
                    FormatLimits::HARD,
                    &mut PreflightBudget::default(),
                )
                .unwrap_err()
                .class(),
                FormatErrorClass::NonCanonicalOrder
            );
        }
    }

    #[test]
    fn discriminated_rows_enforce_lfsm_and_lfsd_presence_matrices() {
        let source_location =
            &portable_object_schema(PortableObjectKind::SourceMap).sections[1].tables[2];
        let valid_text = encoded_variant_row(source_location.row, &[1, 2, 3, 4, 5, 6, 7, 8], 2, 0);
        let table = table_with_rows(source_location, &[valid_text]);
        preflight_table_with_registry(
            &table,
            source_location,
            FormatLimits::HARD,
            &mut PreflightBudget::default(),
        )
        .unwrap();

        let text_with_road_field =
            encoded_variant_row(source_location.row, &[1, 2, 3, 4, 5, 6, 7, 8, 9], 2, 0);
        let table = table_with_rows(source_location, &[text_with_road_field]);
        assert_eq!(
            preflight_table_with_registry(
                &table,
                source_location,
                FormatLimits::HARD,
                &mut PreflightBudget::default(),
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::BindingMismatch
        );

        let static_rule =
            &portable_object_schema(PortableObjectKind::SemanticDiff).sections[4].tables[0];
        let valid_modify = encoded_variant_row(static_rule.row, &[1, 2, 4, 6, 9], 1, 0);
        let table = table_with_rows(static_rule, &[valid_modify]);
        preflight_table_with_registry(
            &table,
            static_rule,
            FormatLimits::HARD,
            &mut PreflightBudget::default(),
        )
        .unwrap();

        let no_payload = encoded_variant_row(static_rule.row, &[1, 2, 4, 6], 1, 0);
        let table = table_with_rows(static_rule, &[no_payload]);
        assert_eq!(
            preflight_table_with_registry(
                &table,
                static_rule,
                FormatLimits::HARD,
                &mut PreflightBudget::default(),
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::BindingMismatch
        );

        let wrong_code = encoded_variant_row(static_rule.row, &[1, 2, 4, 6, 9], 1, 2);
        let table = table_with_rows(static_rule, &[wrong_code]);
        assert_eq!(
            preflight_table_with_registry(
                &table,
                static_rule,
                FormatLimits::HARD,
                &mut PreflightBudget::default(),
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::UnknownKind
        );

        let spatial_configuration =
            &portable_object_schema(PortableObjectKind::SemanticDiff).sections[5].tables[0];
        let initialize = encoded_variant_row(spatial_configuration.row, &[1, 3], 1, 0);
        let table = table_with_rows(spatial_configuration, &[initialize.clone(), initialize]);
        assert_eq!(
            preflight_table_with_registry(
                &table,
                spatial_configuration,
                FormatLimits::HARD,
                &mut PreflightBudget::default(),
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::BindingMismatch
        );
    }

    #[test]
    fn every_discriminated_registry_variant_rejects_missing_forbidden_and_unknown_shapes() {
        for object_kind in PortableObjectKind::ALL {
            for section in portable_object_schema(object_kind).sections {
                for table_schema in section.tables {
                    let PortableRowShape::DiscriminatedU8 { tag, variants } =
                        table_schema.row.shape
                    else {
                        continue;
                    };

                    for variant in variants {
                        let at_least_one = (variant.at_least_one_field != 0)
                            .then(|| variant.at_least_one_field.trailing_zeros() as u16);
                        let valid_mask = variant.required_fields
                            | at_least_one.map_or(0, |field| 1_u32 << field);
                        let valid_tags = table_schema
                            .row
                            .fields
                            .iter()
                            .filter(|field| valid_mask & (1_u32 << field.tag) != 0)
                            .map(|field| field.tag)
                            .collect::<Vec<_>>();
                        let valid = encoded_variant_row(
                            table_schema.row,
                            &valid_tags,
                            tag,
                            variant.discriminant,
                        );
                        let table = table_with_rows(table_schema, &[valid]);
                        preflight_table_with_registry(
                            &table,
                            table_schema,
                            FormatLimits::HARD,
                            &mut PreflightBudget::default(),
                        )
                        .unwrap_or_else(|error| {
                            panic!(
                                "{object_kind:?} section {} table {} variant {} rejected: {error:?}",
                                section.kind, table_schema.kind, variant.discriminant
                            )
                        });

                        for missing in table_schema.row.fields.iter().filter(|field| {
                            field.tag != tag && variant.required_fields & (1_u32 << field.tag) != 0
                        }) {
                            let tags = valid_tags
                                .iter()
                                .copied()
                                .filter(|field| *field != missing.tag)
                                .collect::<Vec<_>>();
                            let row = encoded_variant_row(
                                table_schema.row,
                                &tags,
                                tag,
                                variant.discriminant,
                            );
                            let table = table_with_rows(table_schema, &[row]);
                            assert_eq!(
                                preflight_table_with_registry(
                                    &table,
                                    table_schema,
                                    FormatLimits::HARD,
                                    &mut PreflightBudget::default(),
                                )
                                .unwrap_err()
                                .class(),
                                FormatErrorClass::BindingMismatch,
                                "{object_kind:?} section {} table {} variant {} accepted missing tag {}",
                                section.kind,
                                table_schema.kind,
                                variant.discriminant,
                                missing.tag
                            );
                        }

                        for forbidden in table_schema
                            .row
                            .fields
                            .iter()
                            .filter(|field| variant.allowed_fields & (1_u32 << field.tag) == 0)
                        {
                            let mut tags = valid_tags.clone();
                            tags.push(forbidden.tag);
                            tags.sort_unstable();
                            let row = encoded_variant_row(
                                table_schema.row,
                                &tags,
                                tag,
                                variant.discriminant,
                            );
                            let table = table_with_rows(table_schema, &[row]);
                            assert_eq!(
                                preflight_table_with_registry(
                                    &table,
                                    table_schema,
                                    FormatLimits::HARD,
                                    &mut PreflightBudget::default(),
                                )
                                .unwrap_err()
                                .class(),
                                FormatErrorClass::BindingMismatch,
                                "{object_kind:?} section {} table {} variant {} accepted forbidden tag {}",
                                section.kind,
                                table_schema.kind,
                                variant.discriminant,
                                forbidden.tag
                            );
                        }

                        if variant.at_least_one_field != 0
                            && variant.required_fields & variant.at_least_one_field == 0
                        {
                            let tags = table_schema
                                .row
                                .fields
                                .iter()
                                .filter(|field| variant.required_fields & (1_u32 << field.tag) != 0)
                                .map(|field| field.tag)
                                .collect::<Vec<_>>();
                            let row = encoded_variant_row(
                                table_schema.row,
                                &tags,
                                tag,
                                variant.discriminant,
                            );
                            let table = table_with_rows(table_schema, &[row]);
                            assert_eq!(
                                preflight_table_with_registry(
                                    &table,
                                    table_schema,
                                    FormatLimits::HARD,
                                    &mut PreflightBudget::default(),
                                )
                                .unwrap_err()
                                .class(),
                                FormatErrorClass::BindingMismatch,
                                "{object_kind:?} section {} table {} variant {} accepted an empty alternative set",
                                section.kind,
                                table_schema.kind,
                                variant.discriminant
                            );
                        }
                    }

                    let unknown = (0_u8..=u8::MAX)
                        .find(|candidate| {
                            variants
                                .iter()
                                .all(|variant| variant.discriminant != *candidate)
                        })
                        .expect("a u8 discriminant registry cannot occupy every value");
                    let mut tags = table_schema
                        .row
                        .fields
                        .iter()
                        .filter(|field| variants[0].required_fields & (1_u32 << field.tag) != 0)
                        .map(|field| field.tag)
                        .collect::<Vec<_>>();
                    if variants[0].at_least_one_field != 0
                        && variants[0].required_fields & variants[0].at_least_one_field == 0
                    {
                        tags.push(variants[0].at_least_one_field.trailing_zeros() as u16);
                        tags.sort_unstable();
                    }
                    let row = encoded_variant_row(table_schema.row, &tags, tag, unknown);
                    let table = table_with_rows(table_schema, &[row]);
                    assert_eq!(
                        preflight_table_with_registry(
                            &table,
                            table_schema,
                            FormatLimits::HARD,
                            &mut PreflightBudget::default(),
                        )
                        .unwrap_err()
                        .class(),
                        FormatErrorClass::UnknownKind,
                        "{object_kind:?} section {} table {} accepted unknown discriminant {unknown}",
                        section.kind,
                        table_schema.kind
                    );
                }
            }
        }
    }

    #[test]
    fn record_vector_rows_use_the_field_specific_nested_registry() {
        let identity =
            &portable_object_schema(PortableObjectKind::CanonicalArtifact).sections[1].tables[0];
        let nested_schema = identity.row.fields[3].nested_row.unwrap();
        let incomplete_nested = encoded_row(nested_schema, Some(2));
        let mut record_value = Vec::new();
        record_value.extend_from_slice(&1_u32.to_le_bytes());
        record_value.extend_from_slice(&incomplete_nested);

        let mut top_fields = identity.row.fields[..3]
            .iter()
            .map(|field| encoded_field(field, field.field_type))
            .collect::<Vec<_>>();
        top_fields.push(encoded_field_with_value(
            &identity.row.fields[3],
            PortableFieldType::RecordVector,
            &record_value,
        ));
        let row_length = 16 + top_fields.iter().map(Vec::len).sum::<usize>();
        let mut row = Vec::new();
        row.extend_from_slice(&(row_length as u64).to_le_bytes());
        row.extend_from_slice(&(top_fields.len() as u32).to_le_bytes());
        row.extend_from_slice(&0_u32.to_le_bytes());
        for field in top_fields {
            row.extend_from_slice(&field);
        }
        let table = table_with_rows(identity, &[row]);
        assert_eq!(
            preflight_table_with_registry(
                &table,
                identity,
                FormatLimits::HARD,
                &mut PreflightBudget::default(),
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::BindingMismatch
        );
    }

    #[test]
    fn registry_tables_share_the_object_wide_utf8_budget() {
        let schema = portable_object_schema(PortableObjectKind::CanonicalPublicationDescriptor);
        let first = &schema.sections[1].tables[0];
        let second = &schema.sections[2].tables[0];
        let row_with_utf8 = |row_schema: &PortableRowSchema| {
            let fields = row_schema
                .fields
                .iter()
                .filter(|field| {
                    field.presence == laneflow_static_contract::PortableFieldPresence::Required
                })
                .map(|field| {
                    if field.field_type == PortableFieldType::Utf8 {
                        encoded_field_with_value(field, field.field_type, b"x")
                    } else {
                        encoded_field(field, field.field_type)
                    }
                })
                .collect::<Vec<_>>();
            let length = 16 + fields.iter().map(Vec::len).sum::<usize>();
            let mut row = Vec::new();
            row.extend_from_slice(&(length as u64).to_le_bytes());
            row.extend_from_slice(&(fields.len() as u32).to_le_bytes());
            row.extend_from_slice(&0_u32.to_le_bytes());
            for field in fields {
                row.extend_from_slice(&field);
            }
            row
        };
        let first_table = table_with_rows(first, &[row_with_utf8(first.row)]);
        let second_table = table_with_rows(second, &[row_with_utf8(second.row)]);
        let mut config = FormatLimitConfig::HARD;
        // SourceMapBinding 有一个 UTF-8，PublicationProvenance 有四个；第二张表必须在
        // 读取第一个 value 时越过对象累计预算。
        config.max_total_utf8_bytes = 1;
        let limits = FormatLimits::try_new(config).unwrap();
        let mut budget = PreflightBudget::default();
        preflight_table_with_registry(&first_table, first, limits, &mut budget).unwrap();
        assert_eq!(
            preflight_table_with_registry(&second_table, second, limits, &mut budget).unwrap_err(),
            FormatError::LimitExceeded {
                dimension: LimitDimension::TotalUtf8Bytes,
                actual: 2,
                limit: 1,
            }
        );
    }

    #[test]
    fn caller_can_reduce_lfsm_source_location_rows_independently() {
        let mut config = FormatLimitConfig::HARD;
        config.max_source_location_rows_per_chunk = 4;
        let limits = FormatLimits::try_new(config).unwrap();
        assert_eq!(
            check_source_location_rows(PortableObjectKind::SourceMap, 2, 3, 5, limits),
            Err(FormatError::LimitExceeded {
                dimension: LimitDimension::SourceLocationRowsPerChunk,
                actual: 5,
                limit: 4,
            })
        );
        check_source_location_rows(PortableObjectKind::SourceMap, 2, 2, 5, limits).unwrap();
    }

    #[test]
    fn lfsm_source_location_wire_count_uses_its_independent_limit_before_row_walk() {
        let kind = PortableObjectKind::SourceMap;
        let original = encoded_object(kind);
        let mut sections = (0..kind.section_count())
            .map(|ordinal| {
                let entry = usize::from(OBJECT_PREAMBLE_BYTE_LENGTH)
                    + ordinal as usize * SECTION_DIRECTORY_ENTRY_BYTE_LENGTH as usize;
                let offset = u64::from_le_bytes(original[entry + 8..entry + 16].try_into().unwrap())
                    as usize;
                let length =
                    u64::from_le_bytes(original[entry + 16..entry + 24].try_into().unwrap())
                        as usize;
                original[offset..offset + length].to_vec()
            })
            .collect::<Vec<_>>();
        let source_location_section = |row_count: u32| {
            let mut table = Vec::new();
            table.extend_from_slice(&3_u16.to_le_bytes());
            table.extend_from_slice(&1_u16.to_le_bytes());
            table.extend_from_slice(&row_count.to_le_bytes());
            table.extend_from_slice(&0_u64.to_le_bytes());
            let mut section = vec![0_u8; 88];
            section[0..4].copy_from_slice(&1_u32.to_le_bytes());
            section[4..6].copy_from_slice(&72_u16.to_le_bytes());
            section[8..16].copy_from_slice(&88_u64.to_le_bytes());
            section[16..18].copy_from_slice(&3_u16.to_le_bytes());
            section[18..20].copy_from_slice(&1_u16.to_le_bytes());
            section[28..32].copy_from_slice(&row_count.to_le_bytes());
            section[40..48].copy_from_slice(&88_u64.to_le_bytes());
            section[48..56].copy_from_slice(&(table.len() as u64).to_le_bytes());
            section[56..88].copy_from_slice(&Sha256::digest(&table));
            section.extend_from_slice(&table);
            section
        };

        sections[1] = source_location_section(FORMAT_HARD_MAX_SOURCE_LOCATION_ROWS_PER_CHUNK + 1);
        let hard_plus_one = object_from_sections(kind, &sections);
        assert_eq!(
            preflight_object_registry(&hard_plus_one, kind, FormatLimits::HARD).unwrap_err(),
            FormatError::LimitExceeded {
                dimension: LimitDimension::SourceLocationRowsPerChunk,
                actual: u64::from(FORMAT_HARD_MAX_SOURCE_LOCATION_ROWS_PER_CHUNK) + 1,
                limit: u64::from(FORMAT_HARD_MAX_SOURCE_LOCATION_ROWS_PER_CHUNK),
            }
        );

        let mut config = FormatLimitConfig::HARD;
        config.max_source_location_rows_per_chunk = 4;
        sections[1] = source_location_section(5);
        let caller_plus_one = object_from_sections(kind, &sections);
        assert_eq!(
            preflight_object_registry(
                &caller_plus_one,
                kind,
                FormatLimits::try_new(config).unwrap(),
            )
            .unwrap_err(),
            FormatError::LimitExceeded {
                dimension: LimitDimension::SourceLocationRowsPerChunk,
                actual: 5,
                limit: 4,
            }
        );
    }
}
