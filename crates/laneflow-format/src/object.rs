//! 完整对象的附录 A registry 结构预检。

use laneflow_static_contract::{
    PortableFieldSchema, PortableFieldType, PortableObjectKind, PortableObjectSchema,
    PortableRowSchema, PortableSectionSchema, PortableTableSchema, Sha256Digest, StableId128,
    portable_object_schema,
};

use crate::{
    FormatError, FormatLimits, FormatStructure, LimitDimension, ObjectFramingView,
    SectionFramingView,
    table::{PreflightBudget, preflight_table_with_registry},
    wire::{checked_slice, read_u8, read_u16, read_u32, read_u64},
};

const SECTION_HEADER_BYTES: u64 = 4;
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
    contract_format: u16,
}

impl<'a> RegistryCheckedObjectView<'a> {
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

    pub(crate) const fn contract_format(self) -> u16 {
        self.contract_format
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
        let table_count = read_u32(framing.bytes(), 0, FormatStructure::Section).ok()?;
        Some(RegistryCheckedSectionView {
            framing,
            schema,
            table_count,
        })
    }
}

/// 已完成该节全部 table/row/field registry 结构预检的借用。
#[derive(Clone, Copy, Debug)]
pub struct RegistryCheckedSectionView<'a> {
    framing: SectionFramingView<'a>,
    schema: &'static PortableSectionSchema,
    table_count: u32,
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
        self.table_count
    }

    /// 按 wire 顺序遍历全部已受检 table；每张 table 只推进一次游标。
    #[must_use]
    pub fn tables(self) -> RegistryCheckedTableIter<'a> {
        RegistryCheckedTableIter {
            bytes: self.bytes(),
            schemas: self.schema.tables.iter(),
            cursor: SECTION_HEADER_BYTES,
            remaining: self.table_count(),
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
        let mut cursor = SECTION_HEADER_BYTES;
        for current in 0..=ordinal {
            let rows_byte_length =
                read_u64(self.bytes(), cursor + 8, FormatStructure::Table).ok()?;
            let table_byte_length = TABLE_HEADER_BYTES.checked_add(rows_byte_length)?;
            if current == ordinal {
                let bytes = checked_slice(
                    self.bytes(),
                    cursor,
                    table_byte_length,
                    FormatStructure::Table,
                )
                .ok()?;
                let row_count = read_u32(bytes, 4, FormatStructure::Table).ok()?;
                return Some(RegistryCheckedTableView {
                    bytes,
                    schema,
                    row_count,
                });
            }
            cursor = cursor.checked_add(table_byte_length)?;
        }
        None
    }
}

/// 已按附录 A table schema 完成结构预检的零拷贝借用。
#[derive(Clone, Copy, Debug)]
pub struct RegistryCheckedTableView<'a> {
    bytes: &'a [u8],
    schema: &'static PortableTableSchema,
    row_count: u32,
}

impl<'a> RegistryCheckedTableView<'a> {
    #[must_use]
    pub const fn kind(self) -> u16 {
        self.schema.kind
    }

    #[must_use]
    pub const fn bytes(self) -> &'a [u8] {
        self.bytes
    }

    #[must_use]
    pub const fn row_count(self) -> u32 {
        self.row_count
    }

    /// 按 wire 顺序遍历全部已受检 row；每行只推进一次游标。
    #[must_use]
    pub fn rows(self) -> RegistryCheckedRowIter<'a> {
        RegistryCheckedRowIter {
            bytes: self.bytes,
            schema: self.schema.row,
            cursor: TABLE_HEADER_BYTES,
            remaining: self.row_count(),
        }
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
        let mut cursor = TABLE_HEADER_BYTES;
        for current in 0..=ordinal {
            let row_byte_length = read_u64(self.bytes, cursor, FormatStructure::Row).ok()?;
            if current == ordinal {
                let bytes =
                    checked_slice(self.bytes, cursor, row_byte_length, FormatStructure::Row)
                        .ok()?;
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
            bytes: self.bytes,
            schema: self.row_schema,
            cursor: 4,
            remaining: self.len(),
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
    bytes: &'a [u8],
    schemas: core::slice::Iter<'static, PortableTableSchema>,
    cursor: u64,
    remaining: u32,
}

impl<'a> Iterator for RegistryCheckedTableIter<'a> {
    type Item = RegistryCheckedTableView<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let Some(schema) = self.schemas.next() else {
            self.remaining = 0;
            return None;
        };
        let Some(rows_byte_length) =
            read_u64(self.bytes, self.cursor + 8, FormatStructure::Table).ok()
        else {
            self.remaining = 0;
            return None;
        };
        let Some(table_byte_length) = TABLE_HEADER_BYTES.checked_add(rows_byte_length) else {
            self.remaining = 0;
            return None;
        };
        let Some(bytes) = checked_slice(
            self.bytes,
            self.cursor,
            table_byte_length,
            FormatStructure::Table,
        )
        .ok() else {
            self.remaining = 0;
            return None;
        };
        let Some(row_count) = read_u32(bytes, 4, FormatStructure::Table).ok() else {
            self.remaining = 0;
            return None;
        };
        let Some(next_cursor) = self.cursor.checked_add(table_byte_length) else {
            self.remaining = 0;
            return None;
        };
        self.cursor = next_cursor;
        self.remaining -= 1;
        Some(RegistryCheckedTableView {
            bytes,
            schema,
            row_count,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        remaining_size_hint(self.remaining)
    }
}

impl core::iter::FusedIterator for RegistryCheckedTableIter<'_> {}

/// table 或一层 `RecordVector` 的单游标 wire-order row 迭代器。
#[derive(Clone, Debug)]
pub struct RegistryCheckedRowIter<'a> {
    bytes: &'a [u8],
    schema: &'static PortableRowSchema,
    cursor: u64,
    remaining: u32,
}

impl<'a> Iterator for RegistryCheckedRowIter<'a> {
    type Item = RegistryCheckedRowView<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
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
        expected_kind.format_version(),
        limits,
    )
}

fn preflight_object_registry_at<'a>(
    bytes: &'a [u8],
    expected_kind: PortableObjectKind,
    expected_format_version: u16,
    schema: &'static PortableObjectSchema,
    contract_format: u16,
    limits: FormatLimits,
) -> Result<RegistryCheckedObjectView<'a>, FormatError> {
    let framing = crate::framing::preflight_object_framing_at(
        bytes,
        expected_kind,
        expected_format_version,
        limits,
    )?;

    let mut declared_table_count = 0_u64;
    for (ordinal, section_schema) in schema.sections.iter().enumerate() {
        let ordinal = u32::try_from(ordinal).map_err(|_| FormatError::ArithmeticOverflow {
            structure: FormatStructure::SectionDirectory,
        })?;
        let section = framing
            .section(ordinal)
            .ok_or(FormatError::BindingMismatch {
                structure: FormatStructure::Section,
            })?;
        let section_bytes = section.bytes();
        let table_count = read_u32(section_bytes, 0, FormatStructure::Section)?;
        let expected_table_count = u64::try_from(section_schema.tables.len()).map_err(|_| {
            FormatError::ArithmeticOverflow {
                structure: FormatStructure::Section,
            }
        })?;
        if u64::from(table_count) != expected_table_count {
            return Err(FormatError::LengthMismatch {
                structure: FormatStructure::Section,
                declared: u64::from(table_count),
                actual: expected_table_count,
            });
        }
        declared_table_count = declared_table_count
            .checked_add(u64::from(table_count))
            .ok_or(FormatError::ArithmeticOverflow {
                structure: FormatStructure::Section,
            })?;
    }
    let expected_table_count = u64::from(expected_kind.table_count());
    if declared_table_count != expected_table_count {
        return Err(FormatError::LengthMismatch {
            structure: FormatStructure::Section,
            declared: declared_table_count,
            actual: expected_table_count,
        });
    }

    let mut budget = PreflightBudget::default();
    for (ordinal, section_schema) in schema.sections.iter().enumerate() {
        let ordinal = u32::try_from(ordinal).map_err(|_| FormatError::ArithmeticOverflow {
            structure: FormatStructure::SectionDirectory,
        })?;
        let section = framing
            .section(ordinal)
            .ok_or(FormatError::BindingMismatch {
                structure: FormatStructure::Section,
            })?;
        let section_bytes = section.bytes();

        let mut cursor = SECTION_HEADER_BYTES;
        for table_schema in section_schema.tables {
            let actual_table_kind = read_u16(section_bytes, cursor, FormatStructure::Table)?;
            if actual_table_kind == 0 {
                return Err(FormatError::UnknownKind {
                    structure: FormatStructure::Table,
                    code: u64::from(actual_table_kind),
                });
            }
            if actual_table_kind != table_schema.kind {
                return Err(FormatError::NonCanonicalOrder {
                    structure: FormatStructure::Section,
                    previous: u64::from(table_schema.kind),
                    current: u64::from(actual_table_kind),
                });
            }
            let rows_byte_length = read_u64(
                section_bytes,
                cursor
                    .checked_add(8)
                    .ok_or(FormatError::ArithmeticOverflow {
                        structure: FormatStructure::Table,
                    })?,
                FormatStructure::Table,
            )?;
            let row_count = read_u32(
                section_bytes,
                cursor
                    .checked_add(4)
                    .ok_or(FormatError::ArithmeticOverflow {
                        structure: FormatStructure::Table,
                    })?,
                FormatStructure::Table,
            )?;
            check_source_location_rows(
                expected_kind,
                section_schema.kind,
                table_schema.kind,
                row_count,
                limits,
            )?;
            let table_byte_length = TABLE_HEADER_BYTES.checked_add(rows_byte_length).ok_or(
                FormatError::ArithmeticOverflow {
                    structure: FormatStructure::Table,
                },
            )?;
            let table_bytes = checked_slice(
                section_bytes,
                cursor,
                table_byte_length,
                FormatStructure::Table,
            )?;
            preflight_table_with_registry(table_bytes, table_schema, limits, &mut budget)?;
            cursor =
                cursor
                    .checked_add(table_byte_length)
                    .ok_or(FormatError::ArithmeticOverflow {
                        structure: FormatStructure::Section,
                    })?;
        }
        if cursor != section_bytes.len() as u64 {
            return Err(FormatError::LengthMismatch {
                structure: FormatStructure::Section,
                declared: cursor,
                actual: section_bytes.len() as u64,
            });
        }
    }

    Ok(RegistryCheckedObjectView {
        framing,
        limits,
        schema,
        contract_format,
    })
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
    let limit = limits.max_source_location_rows();
    if row_count > limit {
        return Err(FormatError::LimitExceeded {
            dimension: LimitDimension::SourceLocationRows,
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
        FORMAT_HARD_MAX_SOURCE_LOCATION_ROWS, OBJECT_PREAMBLE_BYTE_LENGTH, PortableFieldSchema,
        PortableFieldType, PortableRowCardinality, PortableRowSchema, PortableRowShape,
        SECTION_DIRECTORY_ENTRY_BYTE_LENGTH, SECTION_FORMAT_VERSION,
    };

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

    fn encoded_object(kind: PortableObjectKind) -> Vec<u8> {
        let schema = portable_object_schema(kind);
        let sections = schema
            .sections
            .iter()
            .map(|section| {
                let mut bytes = Vec::new();
                bytes.extend_from_slice(&(section.tables.len() as u32).to_le_bytes());
                for table in section.tables {
                    bytes.extend_from_slice(&encoded_table(table));
                }
                bytes
            })
            .collect::<Vec<_>>();
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
            bytes[entry + 2..entry + 4].copy_from_slice(&SECTION_FORMAT_VERSION.to_le_bytes());
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
            let checked = preflight_object_registry(&bytes, kind, FormatLimits::HARD).unwrap();
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
    fn every_section_rejects_table_count_minus_or_plus_one_before_table_walk() {
        for kind in PortableObjectKind::ALL {
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
                let exact = u32::try_from(section_schema.tables.len()).unwrap();
                for count in [exact - 1, exact + 1] {
                    let mut bytes = original.clone();
                    bytes[section_offset..section_offset + 4].copy_from_slice(&count.to_le_bytes());
                    assert_eq!(
                        preflight_object_registry(&bytes, kind, FormatLimits::HARD)
                            .unwrap_err()
                            .class(),
                        FormatErrorClass::LengthMismatch,
                        "{kind:?} section {} accepted tableCount={count}",
                        section_schema.kind
                    );
                }
            }
        }
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
        let row_count = FormatLimitConfig::HARD.max_rows_per_table;
        let table_bytes = table_with_repeated_row(schema, &encoded, row_count);
        preflight_table_with_registry(
            &table_bytes,
            schema,
            FormatLimits::HARD,
            &mut PreflightBudget::default(),
        )
        .unwrap();

        let table = RegistryCheckedTableView {
            bytes: &table_bytes,
            schema,
            row_count,
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

        let mut wrong_count = original.clone();
        wrong_count[first_section..first_section + 4].copy_from_slice(&0_u32.to_le_bytes());
        assert_eq!(
            preflight_object_registry(&wrong_count, kind, FormatLimits::HARD)
                .unwrap_err()
                .class(),
            FormatErrorClass::LengthMismatch
        );

        let mut wrong_table_kind = original;
        wrong_table_kind[first_section + 4..first_section + 6]
            .copy_from_slice(&2_u16.to_le_bytes());
        assert_eq!(
            preflight_object_registry(&wrong_table_kind, kind, FormatLimits::HARD)
                .unwrap_err()
                .class(),
            FormatErrorClass::NonCanonicalOrder
        );

        let mut trailing = encoded_object(kind);
        trailing.push(0);
        let new_object_length = trailing.len() as u64;
        trailing[24..32].copy_from_slice(&new_object_length.to_le_bytes());
        let last_entry = usize::from(OBJECT_PREAMBLE_BYTE_LENGTH)
            + 3 * SECTION_DIRECTORY_ENTRY_BYTE_LENGTH as usize;
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
        let entity_section_entry = usize::from(OBJECT_PREAMBLE_BYTE_LENGTH)
            + 2 * SECTION_DIRECTORY_ENTRY_BYTE_LENGTH as usize;
        let entity_section_offset = u64::from_le_bytes(
            wrong_order[entity_section_entry + 8..entity_section_entry + 16]
                .try_into()
                .unwrap(),
        ) as usize;
        wrong_order[entity_section_offset + 4..entity_section_offset + 6]
            .copy_from_slice(&2_u16.to_le_bytes());
        assert_eq!(
            preflight_object_registry(&wrong_order, lfca_kind, FormatLimits::HARD)
                .unwrap_err()
                .class(),
            FormatErrorClass::NonCanonicalOrder
        );

        let mut reserved_kind = encoded_object(lfca_kind);
        let reserved_section_offset = u64::from_le_bytes(
            reserved_kind[entity_section_entry + 8..entity_section_entry + 16]
                .try_into()
                .unwrap(),
        ) as usize;
        reserved_kind[reserved_section_offset + 4..reserved_section_offset + 6]
            .copy_from_slice(&21_u16.to_le_bytes());
        assert_eq!(
            preflight_object_registry(&reserved_kind, lfca_kind, FormatLimits::HARD)
                .unwrap_err()
                .class(),
            FormatErrorClass::NonCanonicalOrder
        );
    }

    #[test]
    fn object_preflight_checks_all_table_counts_before_reading_any_table() {
        let kind = PortableObjectKind::CanonicalArtifact;
        let mut bytes = encoded_object(kind);
        let first_section = kind.first_section_offset() as usize;
        bytes[first_section + 6..first_section + 8].copy_from_slice(&2_u16.to_le_bytes());

        let last_entry = usize::from(OBJECT_PREAMBLE_BYTE_LENGTH)
            + (kind.section_count() as usize - 1) * SECTION_DIRECTORY_ENTRY_BYTE_LENGTH as usize;
        let last_section =
            u64::from_le_bytes(bytes[last_entry + 8..last_entry + 16].try_into().unwrap()) as usize;
        bytes[last_section..last_section + 4].copy_from_slice(&0_u32.to_le_bytes());

        assert_eq!(
            preflight_object_registry(&bytes, kind, FormatLimits::HARD)
                .unwrap_err()
                .class(),
            FormatErrorClass::LengthMismatch
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
        config.max_source_location_rows = 4;
        let limits = FormatLimits::try_new(config).unwrap();
        assert_eq!(
            check_source_location_rows(PortableObjectKind::SourceMap, 2, 3, 5, limits),
            Err(FormatError::LimitExceeded {
                dimension: LimitDimension::SourceLocationRows,
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
        let section_entry = usize::from(OBJECT_PREAMBLE_BYTE_LENGTH)
            + usize::try_from(SECTION_DIRECTORY_ENTRY_BYTE_LENGTH).unwrap();
        let section_offset = u64::from_le_bytes(
            original[section_entry + 8..section_entry + 16]
                .try_into()
                .unwrap(),
        ) as usize;
        let section_bytes = &original[section_offset..];
        let mut table_offset = 4_usize;
        for _ in 0..2 {
            let rows_length = u64::from_le_bytes(
                section_bytes[table_offset + 8..table_offset + 16]
                    .try_into()
                    .unwrap(),
            );
            table_offset += 16 + usize::try_from(rows_length).unwrap();
        }
        let source_location_count_offset = section_offset + table_offset + 4;

        let mut hard_plus_one = original.clone();
        hard_plus_one[source_location_count_offset..source_location_count_offset + 4]
            .copy_from_slice(&(FORMAT_HARD_MAX_SOURCE_LOCATION_ROWS + 1).to_le_bytes());
        assert_eq!(
            preflight_object_registry(&hard_plus_one, kind, FormatLimits::HARD).unwrap_err(),
            FormatError::LimitExceeded {
                dimension: LimitDimension::SourceLocationRows,
                actual: u64::from(FORMAT_HARD_MAX_SOURCE_LOCATION_ROWS) + 1,
                limit: u64::from(FORMAT_HARD_MAX_SOURCE_LOCATION_ROWS),
            }
        );

        let mut config = FormatLimitConfig::HARD;
        config.max_source_location_rows = 4;
        let mut caller_plus_one = original;
        caller_plus_one[source_location_count_offset..source_location_count_offset + 4]
            .copy_from_slice(&5_u32.to_le_bytes());
        assert_eq!(
            preflight_object_registry(
                &caller_plus_one,
                kind,
                FormatLimits::try_new(config).unwrap(),
            )
            .unwrap_err(),
            FormatError::LimitExceeded {
                dimension: LimitDimension::SourceLocationRows,
                actual: 5,
                limit: 4,
            }
        );
    }
}
