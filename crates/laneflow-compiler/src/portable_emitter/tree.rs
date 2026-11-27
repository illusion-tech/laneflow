//! 发射对象树：受账本授予约束的扁平存储（资源账本设计第 3.4 与 6 节）。
//!
//! 一份可移植对象的编码前投影由八段连续存储组成：节、表、顶层行、顶层字段、嵌套记录、
//! 嵌套记录字段、字节载荷（UTF-8、不透明字节与摘要）与序号向量。子节点以区间引用父节点
//! 之下的一段连续元素，因此整棵树只有八笔授予，编码时也只需按同样的计数建立借用输入。
//!
//! 对象树由同一份构造代码运行两遍得到：[`TreeBuilder::counting`] 只累加各段元素数，不分配；
//! 由计数求出的计划准入后，[`TreeBuilder::building`] 按计数取得授予并写入。两遍写入量
//! 不一致属记账缺陷，计划与实际构造因此不存在第二份等价算术。

use std::alloc::Layout;
use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use laneflow_format::{
    FieldWriteInput, FieldWriteValue, FormatLimits, ObjectWriteInput, PreparedObject,
    RowWriteInput, SectionWriteInput, TableWriteInput, prepare_object,
};
use laneflow_static_contract::PortableObjectKind;

use crate::ledger::{AccountingDefect, BoundedVec, LedgerError, PassBudget, PassLabel, Plan};

use super::PortableEmissionError;
use super::budget::Accounting;
#[cfg(test)]
use super::{OwnedObject, OwnedValue};

#[cfg(test)]
mod tests;

/// 同一段存储内的半开区间。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Span {
    start: usize,
    end: usize,
}

impl Span {
    fn of<T>(self, values: &[T]) -> &[T] {
        &values[self.start..self.end]
    }

    fn of_mut<T>(self, values: &mut [T]) -> &mut [T] {
        &mut values[self.start..self.end]
    }
}

/// 字段值。标量就地保存；载荷保存在对象树的字节或序号存储中，嵌套记录向量引用记录存储。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Value {
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    F32(f32),
    /// 与线格式 `PortableFieldType::F64` 对齐；当前由单元测试覆盖构造。
    #[cfg_attr(not(test), allow(dead_code))]
    F64(f64),
    I32(i32),
    StableId128([u8; 16]),
    Sha256(Span),
    Utf8(Span),
    Bytes(Span),
    Ordinals(Span),
    Records(Span),
}

/// 带登记 tag 的字段。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Field {
    tag: u16,
    value: Value,
}

/// 构造一个字段。
pub(super) const fn field(tag: u16, value: Value) -> Field {
    Field { tag, value }
}

/// 节或表：种类与其子节点区间。
#[derive(Clone, Copy, Debug, PartialEq)]
struct Node {
    kind: u16,
    children: Span,
}

/// 顶层行的引用。
#[derive(Clone, Copy, Debug)]
pub(super) struct RowRef(usize);

/// 嵌套记录的引用。
#[derive(Clone, Copy, Debug)]
pub(super) struct RecordRef(usize);

/// 表的引用。
#[derive(Clone, Copy, Debug)]
pub(super) struct TableRef(usize);

/// 节的引用。
#[derive(Clone, Copy, Debug)]
pub(super) struct SectionRef(usize);

/// 一份对象树各段存储的元素数，由计数模式求得。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct TreeShape {
    sections: usize,
    tables: usize,
    rows: usize,
    fields: usize,
    records: usize,
    record_fields: usize,
    bytes: usize,
    ordinals: usize,
}

impl TreeShape {
    /// 对象树八段存储的计划峰值。
    pub(super) fn plan(self) -> Plan {
        Plan::ZERO
            .array::<Node>(self.sections)
            .array::<Node>(self.tables)
            .array::<Span>(self.rows)
            .array::<Field>(self.fields)
            .array::<Span>(self.records)
            .array::<Field>(self.record_fields)
            .array::<u8>(self.bytes)
            .array::<u32>(self.ordinals)
    }

    /// 编码借用输入的计划峰值：每个字段、行、表与节各一项写入输入。
    fn lowering_plan(self) -> Plan {
        Plan::ZERO
            .array::<FieldWriteInput<'_>>(self.record_fields)
            .array::<RowWriteInput<'_>>(self.records)
            .array::<FieldWriteInput<'_>>(self.fields)
            .array::<RowWriteInput<'_>>(self.rows)
            .array::<TableWriteInput<'_>>(self.tables)
            .array::<SectionWriteInput<'_>>(self.sections)
    }
}

/// 计数模式只累加长度；构建模式写入授予的存储。
enum Arena<T> {
    Count(usize),
    Build(BoundedVec<T>),
}

impl<T> Arena<T> {
    fn len(&self) -> usize {
        match self {
            Self::Count(len) => *len,
            Self::Build(values) => values.len(),
        }
    }

    fn push(&mut self, value: T) -> Result<(), LedgerError> {
        match self {
            Self::Count(len) => {
                *len += 1;
                Ok(())
            }
            Self::Build(values) => values.push(value),
        }
    }

    fn build(pass: &PassBudget<'_>, capacity: usize) -> Result<Self, LedgerError> {
        Ok(Self::Build(pass.vec(capacity)?))
    }

    /// 构建模式结束：写入量必须恰好等于计数模式求得的容量。
    fn finish(self, label: PassLabel) -> Result<BoundedVec<T>, LedgerError> {
        match self {
            Self::Build(values) if values.len() == values.capacity() => Ok(values),
            _ => Err(LedgerError::Defect {
                label,
                defect: AccountingDefect::ShapeMismatch,
            }),
        }
    }
}

impl<T: Clone> Arena<T> {
    fn extend_from_slice(&mut self, values: &[T]) -> Result<(), LedgerError> {
        match self {
            Self::Count(len) => {
                *len += values.len();
                Ok(())
            }
            Self::Build(arena) => arena.extend_from_slice(values),
        }
    }
}

struct Arenas {
    sections: Arena<Node>,
    tables: Arena<Node>,
    rows: Arena<Span>,
    fields: Arena<Field>,
    records: Arena<Span>,
    record_fields: Arena<Field>,
    bytes: Arena<u8>,
    ordinals: Arena<u32>,
}

/// 对象树构造上下文。全部方法只取共享引用，构造代码可以在迭代器闭包与字段数组内部
/// 嵌套调用；第一次错误被保留，此后的写入不再生效，由 [`TreeBuilder::finish`] 报告。
pub(super) struct TreeBuilder {
    label: PassLabel,
    arenas: RefCell<Arenas>,
    error: Cell<Option<PortableEmissionError>>,
    row_open: Cell<bool>,
    record_open: Cell<bool>,
}

impl TreeBuilder {
    /// 计数模式：只累加各段存储的元素数，不分配。
    pub(super) fn counting(label: PassLabel) -> Self {
        Self::with(
            label,
            Arenas {
                sections: Arena::Count(0),
                tables: Arena::Count(0),
                rows: Arena::Count(0),
                fields: Arena::Count(0),
                records: Arena::Count(0),
                record_fields: Arena::Count(0),
                bytes: Arena::Count(0),
                ordinals: Arena::Count(0),
            },
        )
    }

    /// 构建模式：按计数模式求得的元素数从 `pass` 取得八笔授予。
    pub(super) fn building(shape: TreeShape, pass: &PassBudget<'_>) -> Result<Self, LedgerError> {
        Ok(Self::with(
            pass.label(),
            Arenas {
                sections: Arena::build(pass, shape.sections)?,
                tables: Arena::build(pass, shape.tables)?,
                rows: Arena::build(pass, shape.rows)?,
                fields: Arena::build(pass, shape.fields)?,
                records: Arena::build(pass, shape.records)?,
                record_fields: Arena::build(pass, shape.record_fields)?,
                bytes: Arena::build(pass, shape.bytes)?,
                ordinals: Arena::build(pass, shape.ordinals)?,
            },
        ))
    }

    const fn with(label: PassLabel, arenas: Arenas) -> Self {
        Self {
            label,
            arenas: RefCell::new(arenas),
            error: Cell::new(None),
            row_open: Cell::new(false),
            record_open: Cell::new(false),
        }
    }

    fn fail(&self, error: impl Into<PortableEmissionError>) {
        let previous = self.error.take();
        self.error.set(previous.or(Some(error.into())));
    }

    fn failed(&self) -> bool {
        let error = self.error.take();
        let failed = error.is_some();
        self.error.set(error);
        failed
    }

    /// 在存储上执行一次写入；已有错误时跳过。
    fn write(&self, write: impl FnOnce(&mut Arenas) -> Result<(), LedgerError>) {
        if self.failed() {
            return;
        }
        if let Err(error) = write(&mut self.arenas.borrow_mut()) {
            self.fail(error);
        }
    }

    fn len(&self, of: impl FnOnce(&Arenas) -> usize) -> usize {
        of(&self.arenas.borrow())
    }

    /// 保存一段 UTF-8 载荷。
    pub(super) fn utf8(&self, value: &str) -> Value {
        Value::Utf8(self.payload(value.as_bytes()))
    }

    /// 保存一段不透明字节载荷。
    pub(super) fn bytes(&self, value: &[u8]) -> Value {
        Value::Bytes(self.payload(value))
    }

    /// 保存一份 SHA-256 摘要。
    pub(super) fn sha256(&self, value: [u8; 32]) -> Value {
        Value::Sha256(self.payload(&value))
    }

    fn payload(&self, value: &[u8]) -> Span {
        let start = self.len(|arenas| arenas.bytes.len());
        self.write(|arenas| arenas.bytes.extend_from_slice(value));
        Span {
            start,
            end: self.len(|arenas| arenas.bytes.len()),
        }
    }

    /// 保存一段有类型序号的原值向量。
    pub(super) fn ordinals_of<K: laneflow_static_contract::OrdinalKind + Copy>(
        &self,
        values: &[laneflow_static_contract::Ordinal<K>],
    ) -> Value {
        self.ordinals(values.iter().map(|value| value.raw()))
    }

    /// 保存一个 `u32` 序号向量。
    pub(super) fn ordinals(&self, values: impl IntoIterator<Item = u32>) -> Value {
        let start = self.len(|arenas| arenas.ordinals.len());
        for value in values {
            self.write(|arenas| arenas.ordinals.push(value));
        }
        Value::Ordinals(Span {
            start,
            end: self.len(|arenas| arenas.ordinals.len()),
        })
    }

    /// 写入一个顶层行；`fields` 可以是字段或可选字段，`None` 跳过。
    pub(super) fn row<F: Into<Option<Field>>>(
        &self,
        fields: impl IntoIterator<Item = F>,
    ) -> RowRef {
        if self.row_open.replace(true) {
            self.fail(PortableEmissionError::InternalBindingMismatch);
        }
        let start = self.len(|arenas| arenas.fields.len());
        for field in fields.into_iter().filter_map(Into::into) {
            self.write(|arenas| arenas.fields.push(field));
        }
        let end = self.len(|arenas| arenas.fields.len());
        self.row_open.set(false);
        let index = self.len(|arenas| arenas.rows.len());
        self.write(|arenas| arenas.rows.push(Span { start, end }));
        RowRef(index)
    }

    /// 写入一条嵌套记录；v1 只允许一层记录向量。`fields` 的形式同 [`Self::row`]。
    pub(super) fn record<F: Into<Option<Field>>>(
        &self,
        fields: impl IntoIterator<Item = F>,
    ) -> RecordRef {
        if self.record_open.replace(true) {
            self.fail(PortableEmissionError::InternalBindingMismatch);
        }
        let start = self.len(|arenas| arenas.record_fields.len());
        for field in fields.into_iter().filter_map(Into::into) {
            if matches!(field.value, Value::Records(_)) {
                self.fail(PortableEmissionError::InternalBindingMismatch);
            }
            self.write(|arenas| arenas.record_fields.push(field));
        }
        let end = self.len(|arenas| arenas.record_fields.len());
        self.record_open.set(false);
        let index = self.len(|arenas| arenas.records.len());
        self.write(|arenas| arenas.records.push(Span { start, end }));
        RecordRef(index)
    }

    /// 由连续写入的嵌套记录组成记录向量值。
    pub(super) fn records(&self, records: impl IntoIterator<Item = RecordRef>) -> Value {
        let empty_at = self.len(|arenas| arenas.records.len());
        Value::Records(self.contiguous(records.into_iter().map(|r| r.0), empty_at))
    }

    /// 由连续写入的顶层行组成一张表。
    pub(super) fn table(&self, kind: u16, rows: impl IntoIterator<Item = RowRef>) -> TableRef {
        let empty_at = self.len(|arenas| arenas.rows.len());
        let children = self.contiguous(rows.into_iter().map(|r| r.0), empty_at);
        let index = self.len(|arenas| arenas.tables.len());
        self.write(|arenas| arenas.tables.push(Node { kind, children }));
        TableRef(index)
    }

    /// 由 `write` 依次写入的顶层行组成一张表；用于需要先做额外计算再逐行写入的表。
    pub(super) fn table_with<E>(
        &self,
        kind: u16,
        write: impl FnOnce() -> Result<(), E>,
    ) -> Result<TableRef, E> {
        let start = self.len(|arenas| arenas.rows.len());
        write()?;
        let end = self.len(|arenas| arenas.rows.len());
        let index = self.len(|arenas| arenas.tables.len());
        self.write(|arenas| {
            arenas.tables.push(Node {
                kind,
                children: Span { start, end },
            })
        });
        Ok(TableRef(index))
    }

    /// 由连续写入的表组成一节。
    pub(super) fn section(
        &self,
        kind: u16,
        tables: impl IntoIterator<Item = TableRef>,
    ) -> SectionRef {
        let empty_at = self.len(|arenas| arenas.tables.len());
        let children = self.contiguous(tables.into_iter().map(|t| t.0), empty_at);
        let index = self.len(|arenas| arenas.sections.len());
        self.write(|arenas| arenas.sections.push(Node { kind, children }));
        SectionRef(index)
    }

    /// 由 `write` 依次写入的表组成一节。
    pub(super) fn section_with<E>(
        &self,
        kind: u16,
        write: impl FnOnce() -> Result<(), E>,
    ) -> Result<SectionRef, E> {
        let start = self.len(|arenas| arenas.tables.len());
        write()?;
        let end = self.len(|arenas| arenas.tables.len());
        let index = self.len(|arenas| arenas.sections.len());
        self.write(|arenas| {
            arenas.sections.push(Node {
                kind,
                children: Span { start, end },
            })
        });
        Ok(SectionRef(index))
    }

    /// 核对引用依次相邻并返回其区间；空序列取当前末尾。
    fn contiguous(&self, mut indices: impl Iterator<Item = usize>, empty_at: usize) -> Span {
        let Some(start) = indices.next() else {
            return Span {
                start: empty_at,
                end: empty_at,
            };
        };
        let mut end = start + 1;
        for index in indices {
            if index != end {
                self.fail(PortableEmissionError::InternalBindingMismatch);
            }
            end = index + 1;
        }
        Span { start, end }
    }

    /// 计数模式结束：返回各段元素数。
    pub(super) fn shape(self) -> Result<TreeShape, PortableEmissionError> {
        if let Some(error) = self.error.take() {
            return Err(error);
        }
        let arenas = self.arenas.into_inner();
        Ok(TreeShape {
            sections: arenas.sections.len(),
            tables: arenas.tables.len(),
            rows: arenas.rows.len(),
            fields: arenas.fields.len(),
            records: arenas.records.len(),
            record_fields: arenas.record_fields.len(),
            bytes: arenas.bytes.len(),
            ordinals: arenas.ordinals.len(),
        })
    }

    /// 构建模式结束：全部节须组成对象的节目录，各段写入量须与计数一致。
    pub(super) fn finish(
        self,
        kind: PortableObjectKind,
        sections: impl IntoIterator<Item = SectionRef>,
    ) -> Result<ObjectTree, PortableEmissionError> {
        let span = self.contiguous(sections.into_iter().map(|s| s.0), 0);
        if let Some(error) = self.error.take() {
            return Err(error);
        }
        let label = self.label;
        let arenas = self.arenas.into_inner();
        if span.start != 0 || span.end != arenas.sections.len() {
            return Err(PortableEmissionError::InternalBindingMismatch);
        }
        Ok(ObjectTree {
            kind,
            sections: arenas.sections.finish(label)?,
            tables: arenas.tables.finish(label)?,
            rows: arenas.rows.finish(label)?,
            fields: arenas.fields.finish(label)?,
            records: arenas.records.finish(label)?,
            record_fields: arenas.record_fields.finish(label)?,
            bytes: arenas.bytes.finish(label)?,
            ordinals: arenas.ordinals.finish(label)?,
        })
    }
}

/// 一份可移植对象的编码前投影，全部存储由账本授予。
pub(super) struct ObjectTree {
    kind: PortableObjectKind,
    sections: BoundedVec<Node>,
    tables: BoundedVec<Node>,
    rows: BoundedVec<Span>,
    fields: BoundedVec<Field>,
    records: BoundedVec<Span>,
    record_fields: BoundedVec<Field>,
    bytes: BoundedVec<u8>,
    ordinals: BoundedVec<u32>,
}

impl ObjectTree {
    /// 对象树持有的受控请求字节。
    pub(super) const fn granted_bytes(&self) -> u64 {
        self.sections.granted_bytes()
            + self.tables.granted_bytes()
            + self.rows.granted_bytes()
            + self.fields.granted_bytes()
            + self.records.granted_bytes()
            + self.record_fields.granted_bytes()
            + self.bytes.granted_bytes()
            + self.ordinals.granted_bytes()
    }

    fn shape(&self) -> TreeShape {
        TreeShape {
            sections: self.sections.len(),
            tables: self.tables.len(),
            rows: self.rows.len(),
            fields: self.fields.len(),
            records: self.records.len(),
            record_fields: self.record_fields.len(),
            bytes: self.bytes.len(),
            ordinals: self.ordinals.len(),
        }
    }

    /// 改写第 `section` 节第 `table` 张表第 `row` 行中 tag 为 `tag` 的摘要字段。
    pub(super) fn set_sha256(
        &mut self,
        section: usize,
        table: usize,
        row: usize,
        tag: u16,
        value: [u8; 32],
    ) -> Result<(), PortableEmissionError> {
        let table = self
            .sections
            .get(section)
            .and_then(|section| section.children.of(&self.tables).get(table))
            .ok_or(PortableEmissionError::InternalBindingMismatch)?;
        let row = table
            .children
            .of(&self.rows)
            .get(row)
            .ok_or(PortableEmissionError::InternalBindingMismatch)?;
        let span = row
            .of(&self.fields)
            .iter()
            .find_map(|field| match field.value {
                Value::Sha256(span) if field.tag == tag => Some(span),
                _ => None,
            })
            .ok_or(PortableEmissionError::InternalBindingMismatch)?;
        span.of_mut(&mut self.bytes).copy_from_slice(&value);
        Ok(())
    }

    fn primitive(&self, value: Value) -> Result<FieldWriteValue<'_>, PortableEmissionError> {
        Ok(match value {
            Value::U8(value) => FieldWriteValue::U8(value),
            Value::U16(value) => FieldWriteValue::U16(value),
            Value::U32(value) => FieldWriteValue::U32(value),
            Value::U64(value) => FieldWriteValue::U64(value),
            Value::F32(value) => FieldWriteValue::F32(value),
            Value::F64(value) => FieldWriteValue::F64(value),
            Value::I32(value) => FieldWriteValue::I32(value),
            Value::StableId128(value) => FieldWriteValue::StableId128(value),
            Value::Sha256(span) => FieldWriteValue::Sha256(
                span.of(&self.bytes)
                    .try_into()
                    .map_err(|_| PortableEmissionError::InternalBindingMismatch)?,
            ),
            Value::Utf8(span) => FieldWriteValue::Utf8(
                std::str::from_utf8(span.of(&self.bytes))
                    .map_err(|_| PortableEmissionError::InternalBindingMismatch)?,
            ),
            Value::Bytes(span) => FieldWriteValue::Bytes(span.of(&self.bytes)),
            Value::Ordinals(span) => FieldWriteValue::OrdinalVectorU32(span.of(&self.ordinals)),
            Value::Records(_) => return Err(PortableEmissionError::InternalBindingMismatch),
        })
    }

    /// 建立编码借用输入并完成格式预检，再以预检得到的精确长度准入编码编译遍。
    ///
    /// 借用输入在第一个记账点的编译遍内授予，预检后作为该遍输出继续存续，直到编码结束。
    /// 编码编译遍的计划由 `encode_plan` 按精确长度求出；`encode` 返回值与随之移出编码
    /// 编译遍、继续存续的授予字节，后者作为该遍输出。
    pub(super) fn encode<T>(
        &self,
        accounting: Accounting<'_>,
        points: (&'static str, &'static str),
        limits: FormatLimits,
        encode_plan: impl FnOnce(u64) -> Plan,
        encode: impl FnOnce(
            PreparedObject<'_>,
            &PassBudget<'_>,
        ) -> Result<(T, u64), PortableEmissionError>,
    ) -> Result<T, PortableEmissionError> {
        let shape = self.shape();
        let lowering = accounting.admit(points.0, shape.lowering_plan())?;
        let mut record_fields = lowering.vec::<FieldWriteInput<'_>>(shape.record_fields)?;
        for field in &self.record_fields {
            record_fields.push(FieldWriteInput {
                tag: field.tag,
                value: self.primitive(field.value)?,
            })?;
        }
        let mut records = lowering.vec::<RowWriteInput<'_>>(shape.records)?;
        for span in &self.records {
            records.push(RowWriteInput {
                fields: span.of(&record_fields),
            })?;
        }
        let mut fields = lowering.vec::<FieldWriteInput<'_>>(shape.fields)?;
        for field in &self.fields {
            let value = match field.value {
                Value::Records(span) => FieldWriteValue::RecordVector(span.of(&records)),
                value => self.primitive(value)?,
            };
            fields.push(FieldWriteInput {
                tag: field.tag,
                value,
            })?;
        }
        let mut rows = lowering.vec::<RowWriteInput<'_>>(shape.rows)?;
        for span in &self.rows {
            rows.push(RowWriteInput {
                fields: span.of(&fields),
            })?;
        }
        let mut tables = lowering.vec::<TableWriteInput<'_>>(shape.tables)?;
        for table in &self.tables {
            tables.push(TableWriteInput {
                kind: table.kind,
                rows: table.children.of(&rows),
            })?;
        }
        let mut sections = lowering.vec::<SectionWriteInput<'_>>(shape.sections)?;
        for section in &self.sections {
            sections.push(SectionWriteInput {
                kind: section.kind,
                tables: section.children.of(&tables),
            })?;
        }
        let prepared = prepare_object(
            ObjectWriteInput {
                kind: self.kind,
                sections: &sections,
            },
            limits,
        )?;
        lowering.finish(
            record_fields.granted_bytes()
                + records.granted_bytes()
                + fields.granted_bytes()
                + rows.granted_bytes()
                + tables.granted_bytes()
                + sections.granted_bytes(),
        )?;
        let pass = accounting.admit(points.1, encode_plan(prepared.byte_len()))?;
        let (value, output) = encode(prepared, &pass)?;
        pass.finish(output)?;
        Ok(value)
    }
}

/// 以计数模式与构建模式各运行一遍 `emit`，在 `point` 编译遍内构造对象树。
///
/// `emit` 返回对象的节目录。计数模式求得的计划在任何与计数成正比的分配前准入；对象树
/// 作为该编译遍的输出继续存续，释放时归还额度。
pub(super) fn build_tree<S, E>(
    accounting: Accounting<'_>,
    point: &'static str,
    kind: PortableObjectKind,
    emit: impl Fn(&TreeBuilder) -> Result<S, E>,
) -> Result<ObjectTree, PortableEmissionError>
where
    S: IntoIterator<Item = SectionRef>,
    PortableEmissionError: From<E>,
{
    let counter = TreeBuilder::counting(accounting.label(point));
    emit(&counter)?;
    let shape = counter.shape()?;
    let pass = accounting.admit(point, shape.plan())?;
    let builder = TreeBuilder::building(shape, &pass)?;
    let sections = emit(&builder)?;
    let tree = builder.finish(kind, sections)?;
    pass.finish(tree.granted_bytes())?;
    Ok(tree)
}

/// `Arc<[u8]>` 的引用计数分配请求：两个计数器与载荷。
pub(super) fn arc_bytes_layout(length: usize) -> Option<Layout> {
    Layout::new::<[AtomicUsize; 2]>()
        .extend(Layout::array::<u8>(length).ok()?)
        .ok()
        .map(|(layout, _)| layout.pad_to_align())
}

/// 按授予直接创建最终共享字节，初始由调用方唯一持有并可安全原位编码。
pub(super) fn zeroed_shared_bytes(
    pass: &PassBudget<'_>,
    length: usize,
) -> Result<(Arc<[u8]>, crate::ledger::Grant), PortableEmissionError> {
    let layout = arc_bytes_layout(length).ok_or(PortableEmissionError::ArithmeticOverflow)?;
    let grant = pass.grant_layout(layout)?;
    // RepeatN 实现 TrustedLen；Arc::from_iter 对精确定长迭代器保证单次分配。
    // 不改成普通迭代器或先收集 Vec，避免重新引入未授予的完整中间缓冲（#1441）。
    Ok((Arc::from_iter(std::iter::repeat_n(0_u8, length)), grant))
}

#[cfg(test)]
impl ObjectTree {
    /// 还原为拥有型嵌套对象，供测试改写后重新编码。
    pub(super) fn to_owned_object(&self) -> OwnedObject {
        let value = |value: Value| -> OwnedValue {
            match value {
                Value::U8(value) => OwnedValue::U8(value),
                Value::U16(value) => OwnedValue::U16(value),
                Value::U32(value) => OwnedValue::U32(value),
                Value::U64(value) => OwnedValue::U64(value),
                Value::F32(value) => OwnedValue::F32(value),
                Value::F64(value) => OwnedValue::F64(value),
                Value::I32(value) => OwnedValue::I32(value),
                Value::StableId128(value) => OwnedValue::StableId128(value),
                Value::Sha256(span) => {
                    OwnedValue::Sha256(span.of(&self.bytes).try_into().expect("摘要为 32 字节"))
                }
                Value::Utf8(span) => OwnedValue::Utf8(
                    std::str::from_utf8(span.of(&self.bytes))
                        .expect("对象树只保存 UTF-8 字符串")
                        .into(),
                ),
                Value::Bytes(span) => OwnedValue::Bytes(span.of(&self.bytes).into()),
                Value::Ordinals(span) => {
                    OwnedValue::OrdinalVectorU32(span.of(&self.ordinals).into())
                }
                Value::Records(_) => unreachable!("记录向量在外层展开"),
            }
        };
        let fields = |span: Span, arena: &[Field]| -> Box<[super::OwnedField]> {
            span.of(arena)
                .iter()
                .map(|field| super::OwnedField {
                    tag: field.tag,
                    value: match field.value {
                        Value::Records(span) => OwnedValue::RecordVector(
                            span.of(&self.records)
                                .iter()
                                .map(|record| super::OwnedRow {
                                    fields: record
                                        .of(&self.record_fields)
                                        .iter()
                                        .map(|nested| super::OwnedField {
                                            tag: nested.tag,
                                            value: value(nested.value),
                                        })
                                        .collect(),
                                })
                                .collect(),
                        ),
                        other => value(other),
                    },
                })
                .collect()
        };
        OwnedObject {
            kind: self.kind,
            sections: self
                .sections
                .iter()
                .map(|section| super::OwnedSection {
                    kind: section.kind,
                    tables: section
                        .children
                        .of(&self.tables)
                        .iter()
                        .map(|table| super::OwnedTable {
                            kind: table.kind,
                            rows: table
                                .children
                                .of(&self.rows)
                                .iter()
                                .map(|row| super::OwnedRow {
                                    fields: fields(*row, &self.fields),
                                })
                                .collect(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}

/// 测试用：把拥有型嵌套对象复制进对象树并编码为内存字节。
#[cfg(test)]
pub(super) fn encode_owned_object(
    object: &OwnedObject,
    limits: FormatLimits,
) -> Result<Box<[u8]>, PortableEmissionError> {
    let accounting = super::budget::test_accounting(u64::MAX);
    let tree = build_tree(accounting, "test-tree", object.kind, |builder| {
        Ok::<_, PortableEmissionError>(builder.copy_owned(object))
    })?;
    tree.encode(
        accounting,
        ("test-lowering", "test-encode"),
        limits,
        |length| Plan::ZERO.array::<u8>(usize::try_from(length).unwrap_or(usize::MAX)),
        |prepared, pass| {
            let length = usize::try_from(prepared.byte_len())
                .map_err(|_| PortableEmissionError::ArithmeticOverflow)?;
            let mut buffer = pass.vec::<u8>(length)?;
            buffer.resize(length, 0)?;
            laneflow_format::encode_prepared_object(prepared, &mut buffer)?;
            Ok((Box::from(&buffer[..]), 0))
        },
    )
}

#[cfg(test)]
impl TreeBuilder {
    /// 把拥有型嵌套对象逐节复制进对象树，返回其节目录。
    ///
    /// 测试以拥有型嵌套对象构造输入后，经此复制进对象树再编码。
    pub(super) fn copy_owned(
        &self,
        object: &OwnedObject,
    ) -> impl Iterator<Item = SectionRef> + use<> {
        let first = self.len(|arenas| arenas.sections.len());
        for section in &object.sections {
            self.section(
                section.kind,
                section.tables.iter().map(|table| {
                    self.table(
                        table.kind,
                        table.rows.iter().map(|row| {
                            self.row(
                                row.fields
                                    .iter()
                                    .map(|owned| field(owned.tag, self.copy_value(&owned.value))),
                            )
                        }),
                    )
                }),
            );
        }
        (first..self.len(|arenas| arenas.sections.len())).map(SectionRef)
    }

    fn copy_value(&self, value: &OwnedValue) -> Value {
        match value {
            OwnedValue::U8(value) => Value::U8(*value),
            OwnedValue::U16(value) => Value::U16(*value),
            OwnedValue::U32(value) => Value::U32(*value),
            OwnedValue::U64(value) => Value::U64(*value),
            OwnedValue::F32(value) => Value::F32(*value),
            OwnedValue::F64(value) => Value::F64(*value),
            OwnedValue::I32(value) => Value::I32(*value),
            OwnedValue::StableId128(value) => Value::StableId128(*value),
            OwnedValue::Sha256(value) => self.sha256(*value),
            OwnedValue::Utf8(value) => self.utf8(value),
            OwnedValue::Bytes(value) => self.bytes(value),
            OwnedValue::OrdinalVectorU32(values) => self.ordinals(values.iter().copied()),
            OwnedValue::RecordVector(rows) => self.records(rows.iter().map(|row| {
                self.record(
                    row.fields
                        .iter()
                        .map(|owned| field(owned.tag, self.copy_value(&owned.value))),
                )
            })),
        }
    }
}
