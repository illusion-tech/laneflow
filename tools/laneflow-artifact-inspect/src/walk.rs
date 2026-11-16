//! 借用遍历的字段路径、物理位置与登记名称解析。
//!
//! 路径按「节登记名.表登记名[零基全表逻辑行].字段登记名」组织，嵌套向量与
//! LFSD 已登记内嵌行沿完整路径展开。物理位置是值载荷在对象 exact bytes 内的
//! 绝对偏移，由同一分配内的借用指针差得出，不复制线格式偏移常量。

use laneflow_format::{
    RegistryCheckedRowView, RegistryCheckedSectionView, RegistryCheckedTableView,
};
use laneflow_static_contract::{PortableObjectKind, PortableRowShape, portable_object_schema};

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

/// 一个字段的完整逻辑路径与物理位置。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldLocation {
    /// 零基节序号。
    pub section_ordinal: u32,
    /// 节登记名。
    pub section_name: &'static str,
    /// 表登记 kind。
    pub table_kind: u16,
    /// 表登记名。
    pub table_name: &'static str,
    /// 零基全表逻辑行序号。
    pub logical_row: u32,
    /// 字段登记 tag。
    pub field_tag: u16,
    /// 字段登记名。
    pub field_name: &'static str,
    /// 嵌套步骤（空 = 顶层字段）。
    pub nested: Vec<NestedStep>,
    /// 值载荷在对象 exact bytes 内的绝对偏移（适用时）。
    pub object_offset: Option<u64>,
}

impl FieldLocation {
    /// 内嵌行内部的根路径（继承外层行与成员步骤，字段信息由后续步骤给出）。
    #[must_use]
    pub fn root() -> Self {
        Self {
            section_ordinal: 0,
            section_name: "",
            table_kind: 0,
            table_name: "",
            logical_row: 0,
            field_tag: 0,
            field_name: "",
            nested: Vec::new(),
            object_offset: None,
        }
    }

    /// 构造顶层字段路径。
    #[must_use]
    pub fn top_level(
        section: &SectionIdentity,
        table: &TableIdentity,
        logical_row: u32,
        field_tag: u16,
        field_name: &'static str,
        object_offset: Option<u64>,
    ) -> Self {
        Self {
            section_ordinal: section.ordinal,
            section_name: section.name,
            table_kind: table.kind,
            table_name: table.name,
            logical_row,
            field_tag,
            field_name,
            nested: Vec::new(),
            object_offset,
        }
    }

    /// 复制当前路径并进入一个 RecordVector 项。
    #[must_use]
    pub fn clone_record_vector_item(&self, index: u32) -> Self {
        let mut nested = self.clone();
        nested.nested.push(NestedStep::RecordVectorItem {
            index,
            field_tag: self.field_tag,
            field_name: self.field_name,
        });
        nested
    }

    /// 渲染为点分路径文本（确定性）：外层字段先行，下标/成员标注随嵌套步骤，
    /// 叶字段名收尾，例如 `SourceModules.SourceLocation[7].startLine`、
    /// `Sec.CanonicalIdentity[0].identityFields[1].value`。
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = format!(
            "{}.{}[{}]",
            self.section_name, self.table_name, self.logical_row
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
pub fn field_name(
    row_schema: &laneflow_static_contract::PortableRowSchema,
    tag: u16,
) -> Option<&'static str> {
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
        .is_some_and(|table| table.row.shape == PortableRowShape::PolicyLocalChange)
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

/// 值载荷在对象 exact bytes 内的绝对偏移：同一分配内的借用指针差。
#[must_use]
pub fn object_offset(object_bytes: &[u8], value_bytes: &[u8]) -> u64 {
    let base = object_bytes.as_ptr() as usize;
    let value = value_bytes.as_ptr() as usize;
    u64::try_from(value.saturating_sub(base)).unwrap_or(0)
}
