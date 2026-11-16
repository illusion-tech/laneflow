//! 借用遍历：字段联接键、双侧顺序游标与登记名称解析。
//!
//! 遍历契约只有一个入口：[`SideWalk`] 按节→表→行顺序推进，行与字段只经
//! `rows()` / `fields()` 单游标迭代器访问（不做 `row(i)` 随机重扫）；物理
//! 位置由本侧对象字节与借用切片的指针差给出，两侧各自计算、互不共用。
//! [`FieldPath`] 是纯逻辑联接键（节/表/逻辑行/嵌套步骤/tag/登记名），不携带
//! 任何单侧物理事实——那是 `FieldSide` 的职责。

use laneflow_format::{
    RegistryCheckedFieldView, RegistryCheckedRowView, RegistryCheckedSectionView,
    RegistryCheckedTableView,
};
use laneflow_static_contract::{PortableObjectKind, PortableRowSchema, portable_object_schema};

/// 路径中的嵌套步骤。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NestedStep {
    /// 进入 RecordVector 的第 `index` 项（零基），并在其中定位字段。
    RecordVectorItem {
        /// 向量项零基下标。
        index: u32,
        /// 外层字段的登记 tag。
        field_tag: u16,
        /// 外层字段的登记名。
        field_name: &'static str,
    },
    /// 进入 LFSD 已登记 Bytes 内嵌行的字段。
    PolicyMember {
        /// 成员种类（LFSD 变更行 tag 3 的值）。
        kind: u8,
        /// 外层字段的登记 tag（5 或 6）。
        field_tag: u16,
        /// 外层字段的登记名。
        field_name: &'static str,
    },
}

/// 字段的完整逻辑路径（联接键）：外层字段先行，下标/成员标注随嵌套步骤，
/// 叶字段名收尾。渲染形如 `SourceModules.SourceLocation[7].startLine`、
/// `Sec.CanonicalIdentity[0].identityFields[1].value`。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldPath {
    /// 节身份。
    pub section: SectionIdentity,
    /// 表身份。
    pub table: TableIdentity,
    /// 零基全表逻辑行序号。
    pub logical_row: u32,
    /// 叶字段登记 tag。
    pub field_tag: u16,
    /// 叶字段登记名。
    pub field_name: &'static str,
    /// 嵌套步骤（空 = 顶层字段）。
    pub nested: Vec<NestedStep>,
}

impl FieldPath {
    /// 渲染为点分路径文本（确定性）。
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = format!(
            "{}.{}[{}]",
            self.section.name, self.table.name, self.logical_row
        );
        for step in &self.nested {
            match step {
                NestedStep::RecordVectorItem {
                    index, field_name, ..
                } => {
                    out.push('.');
                    out.push_str(field_name);
                    out.push_str(&format!("[{index}]"));
                }
                NestedStep::PolicyMember {
                    kind, field_name, ..
                } => {
                    out.push('.');
                    out.push_str(field_name);
                    out.push_str(&format!("{{member={kind}}}"));
                }
            }
        }
        out.push('.');
        out.push_str(self.field_name);
        out
    }

    /// 追加一个嵌套步骤后的路径副本。
    #[must_use]
    pub fn with_step(&self, step: NestedStep) -> Self {
        let mut nested = self.clone();
        nested.nested.push(step);
        nested
    }
}

/// 节的身份信息（零基序号 + 登记名）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SectionIdentity {
    /// 零基节序号。
    pub ordinal: u32,
    /// 节登记名。
    pub name: &'static str,
}

/// 表的身份信息（登记 kind + 登记名）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TableIdentity {
    /// 表登记 kind。
    pub kind: u16,
    /// 表登记名。
    pub name: &'static str,
}

/// 与受检节视图并列的登记身份（同一零基序号）。
#[must_use]
pub fn section_identity(
    kind: PortableObjectKind,
    section: &RegistryCheckedSectionView<'_>,
    ordinal: u32,
) -> SectionIdentity {
    let name = portable_object_schema(kind)
        .sections
        .get(usize::try_from(ordinal).unwrap_or(usize::MAX))
        .map_or("", |schema| schema.name);
    debug_assert_eq!(section.kind(), u16::try_from(ordinal + 1).unwrap_or(0));
    SectionIdentity { ordinal, name }
}

/// 与受检表视图并列的登记身份（同一零基序号）。
#[must_use]
pub fn table_identity(
    kind: PortableObjectKind,
    section_ordinal: u32,
    table: &RegistryCheckedTableView<'_>,
    table_ordinal: u32,
) -> TableIdentity {
    let name = portable_object_schema(kind)
        .sections
        .get(usize::try_from(section_ordinal).unwrap_or(usize::MAX))
        .and_then(|section| {
            section
                .tables
                .get(usize::try_from(table_ordinal).unwrap_or(usize::MAX))
        })
        .map_or("", |schema| schema.name);
    TableIdentity {
        kind: table.kind(),
        name,
    }
}

/// 按 tag 解析行 schema 中的字段登记名；未知 tag 返回 `None`。
#[must_use]
pub fn field_name(row_schema: &PortableRowSchema, tag: u16) -> Option<&'static str> {
    row_schema
        .fields
        .iter()
        .find(|field| field.tag == tag)
        .map(|field| field.name)
}

/// 判定某节某表的行形状是否为 LFSD 策略局部变更（内嵌 RowV1 成员）。
#[must_use]
pub fn is_policy_local_change_table(
    kind: PortableObjectKind,
    section_ordinal: u32,
    table_ordinal: u32,
) -> bool {
    portable_object_schema(kind)
        .sections
        .get(usize::try_from(section_ordinal).unwrap_or(usize::MAX))
        .and_then(|section| {
            section
                .tables
                .get(usize::try_from(table_ordinal).unwrap_or(usize::MAX))
        })
        .is_some_and(|table| {
            table.row.shape == laneflow_static_contract::PortableRowShape::PolicyLocalChange
        })
}

/// 从 LFSD 策略局部变更行提取成员种类（tag 3 的 `U8` 值）。
#[must_use]
pub fn policy_member_kind(row: RegistryCheckedRowView<'_>) -> Option<u8> {
    let field = row.field_by_tag(3)?;
    match field.value() {
        Ok(laneflow_format::RegistryCheckedFieldValue::U8(kind)) => Some(kind),
        _ => None,
    }
}

/// 一个对象侧的顺序游标：行遍历契约与物理偏移的唯一载体。
///
/// - 行只经 [`RegistryCheckedTableView::rows`] 单游标顺序推进，随机访问
///   `row(i)` 的重复扫描在本层没有入口。
/// - 物理位置 = 本侧对象 exact bytes 与借用切片起点的指针差（同一分配内，
///   safe 计算）；两侧各持一个 `SideWalk`，偏移天然各自正确。
#[derive(Clone, Copy)]
pub struct SideWalk<'a> {
    object_bytes: &'a [u8],
}

impl<'a> SideWalk<'a> {
    /// 绑定一个对象侧。
    #[must_use]
    pub const fn new(object_bytes: &'a [u8]) -> Self {
        Self { object_bytes }
    }

    /// 本侧对象 exact bytes。
    #[must_use]
    pub const fn object_bytes(self) -> &'a [u8] {
        self.object_bytes
    }

    /// 按顺序枚举一张表的（零基逻辑行，行视图）。
    pub fn table_rows(
        self,
        table: RegistryCheckedTableView<'a>,
    ) -> impl Iterator<Item = (u32, RegistryCheckedRowView<'a>)> {
        table
            .rows()
            .enumerate()
            .map(|(index, row)| (u32::try_from(index).unwrap_or(u32::MAX), row))
    }

    /// 一个值载荷在本侧对象内的绝对偏移。
    #[must_use]
    pub fn value_offset(self, field: RegistryCheckedFieldView<'_>) -> u64 {
        // 借用切片与本侧对象同属一个分配：指针差是 safe 且精确的。
        let base = self.object_bytes.as_ptr() as usize;
        let value = field.value_bytes().as_ptr() as usize;
        u64::try_from(value.saturating_sub(base)).unwrap_or(0)
    }

    /// 一行在本侧对象内的绝对偏移。
    #[must_use]
    pub fn row_offset(self, row: RegistryCheckedRowView<'_>) -> u64 {
        let base = self.object_bytes.as_ptr() as usize;
        let start = row.bytes().as_ptr() as usize;
        u64::try_from(start.saturating_sub(base)).unwrap_or(0)
    }
}
