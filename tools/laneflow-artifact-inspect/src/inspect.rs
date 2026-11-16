//! 单对象检视：识别、预检、有界摘要与按登记展开。
//!
//! 检查语义完全复用 `laneflow-format::preflight_object_registry`：framing、
//! registry 与直接值域三层预检在无摘要探针的闭包内执行；本模块只组织摘要
//! 与展开数据，不解释未受检内容。

use laneflow_format::{FormatError, FormatLimits, RegistryCheckedObjectView, without_digest_probe};
use laneflow_static_contract::{PortableObjectKind, Sha256Digest};

use crate::budget::ReportLimits;
use crate::value::{ValueDisplay, display_field_value, embedded_row_display};
use crate::walk::{
    FieldLocation, SectionIdentity, TableIdentity, field_name, is_policy_local_change_table,
    object_offset, policy_member_kind, section_identity, table_identity,
};

/// 检视展开选项。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExpandSet {
    /// 展开对象前导的逐字段值。
    pub preamble: bool,
    /// 展开节目录明细。
    pub sections: bool,
    /// 展开每表 chunk 目录明细。
    pub chunks: bool,
    /// 按登记名称展开字段值（受行范围与行数限额约束）。
    pub fields: bool,
}

/// 行范围：零基全表逻辑行，左闭右开。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RowRange {
    /// 起始行（含）。
    pub start: u32,
    /// 结束行（不含）。
    pub end: u32,
}

/// 检视筛选。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InspectFilters {
    /// 按零基序号或登记名筛选节；`None` 为全部。
    pub section: Option<SectionSelector>,
    /// 按 kind 数值或登记名筛选表；`None` 为全部。
    pub table: Option<TableSelector>,
    /// 行范围（作用于每张表的零基逻辑行）。
    pub rows: Option<RowRange>,
}

/// 节选择器。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SectionSelector {
    /// 零基节序号。
    Ordinal(u32),
    /// 节登记名（区分大小写）。
    Name(String),
}

/// 表选择器。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TableSelector {
    /// 表登记 kind。
    Kind(u16),
    /// 表登记名（区分大小写）。
    Name(String),
}

/// 检视选项。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InspectOptions {
    /// 展开集。
    pub expand: ExpandSet,
    /// 筛选。
    pub filters: InspectFilters,
}

/// 一个物理 chunk 的目录摘要。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkSummary {
    /// 目录内 chunk 序号（同表从 0 连续）。
    pub index: u32,
    /// 首个零基全表逻辑行。
    pub first_logical_row: u32,
    /// chunk 内行数。
    pub row_count: u32,
    /// 相对 section 起点的 byte offset。
    pub byte_offset: u64,
    /// chunk exact byte length。
    pub byte_length: u64,
    /// 目录登记的 SHA-256 摘要（LFCP singleton 无）。
    pub digest: Option<Sha256Digest>,
}

/// 一张逻辑表的摘要。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableSummary {
    /// 表身份。
    pub identity: TableIdentity,
    /// 跨全部 chunk 的累计行数。
    pub row_count: u32,
    /// 物理 chunk 数；LFCP singleton 固定为一且无 chunk 目录。
    pub chunk_count: u32,
    /// 该节是否没有 chunk 目录（LFCP singleton）。
    pub no_chunk_directory: bool,
    /// chunk 目录明细。
    pub chunks: Vec<ChunkSummary>,
}

/// 一个节的摘要。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SectionSummary {
    /// 节身份。
    pub identity: SectionIdentity,
    /// 节线格式版本。
    pub format_version: u16,
    /// 相对对象起点的 byte offset。
    pub byte_offset: u64,
    /// exact byte length。
    pub byte_length: u64,
    /// 逻辑表数。
    pub table_count: u32,
    /// 累计行数。
    pub row_count: u64,
    /// 表摘要。
    pub tables: Vec<TableSummary>,
}

/// 一条展开的字段值。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldEntry {
    /// 字段路径与物理位置。
    pub location: FieldLocation,
    /// 值展示。
    pub value: ValueDisplay,
}

/// 检视报告。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InspectReport {
    /// 已识别并通过预检的对象种类。
    pub kind: PortableObjectKind,
    /// 对象格式版本。
    pub format_version: u16,
    /// 对象 exact bytes 长度。
    pub object_byte_length: u64,
    /// 节摘要（含表与 chunk 概览）。
    pub sections: Vec<SectionSummary>,
    /// 展开的字段条目（受筛选与限额约束）。
    pub fields: Vec<FieldEntry>,
    /// 字段展开因行数限额被截断的表（表名）与剩余行数。
    pub truncated_tables: Vec<(TableIdentity, u32)>,
}

impl InspectReport {
    /// 逻辑表总数。
    #[must_use]
    pub fn table_count(&self) -> u32 {
        self.sections
            .iter()
            .map(|section| section.table_count)
            .sum()
    }

    /// 全对象总行数。
    #[must_use]
    pub fn row_count(&self) -> u64 {
        self.sections.iter().map(|section| section.row_count).sum()
    }

    /// 物理 chunk 总数；LFCP 无 chunk 目录时返回 `None`。
    #[must_use]
    pub fn chunk_count(&self) -> Option<u32> {
        if self
            .sections
            .iter()
            .any(|section| section.tables.iter().any(|table| table.no_chunk_directory))
        {
            return None;
        }
        Some(
            self.sections
                .iter()
                .flat_map(|section| section.tables.iter())
                .map(|table| table.chunk_count)
                .sum(),
        )
    }
}

/// 检视错误。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InspectError {
    /// 输入短于对象前导。
    TooShortForMagic {
        /// 实际字节数。
        actual: usize,
    },
    /// magic 无法识别为当前支持的对象种类。
    UnknownMagic {
        /// 前 4 个字节（不足 4 时补齐展示）。
        magic: Vec<u8>,
    },
    /// 正式预检失败。
    Preflight(FormatError),
}

impl core::fmt::Display for InspectError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooShortForMagic { actual } => {
                write!(
                    f,
                    "输入短于 32 字节对象前导（{actual} 字节），无法识别对象种类"
                )
            }
            Self::UnknownMagic { magic } => {
                write!(
                    f,
                    "未知对象种类（magic={}）",
                    crate::value::hex_string(magic, 8)
                )
            }
            Self::Preflight(error) => write!(f, "单对象结构与直接值域预检失败：{error:?}"),
        }
    }
}

impl std::error::Error for InspectError {}

/// 对完整对象执行检视：识别、预检、摘要与（可选）字段展开。
///
/// 预检在 `without_digest_probe` 内执行，损坏输入不写 #794 取证转储。
/// 展示限额只影响 `fields` 展开条数；摘要与检查结论不受影响。
///
/// # Errors
///
/// 输入无法识别为当前支持的对象种类，或正式预检失败时返回 [`InspectError`]。
pub fn inspect(
    bytes: &[u8],
    options: &InspectOptions,
    format_limits: FormatLimits,
    report_limits: &ReportLimits,
) -> Result<InspectReport, InspectError> {
    let view = preflight_view(bytes, format_limits)?;
    let mut expand_rows_budget = report_limits.max_expanded_rows;
    Ok(build_report(
        view,
        options,
        report_limits,
        format_limits,
        &mut expand_rows_budget,
    ))
}

/// 识别并预检对象，返回受检视图。
fn preflight_view(
    bytes: &[u8],
    format_limits: FormatLimits,
) -> Result<RegistryCheckedObjectView<'_>, InspectError> {
    if bytes.len() < 32 {
        return Err(InspectError::TooShortForMagic {
            actual: bytes.len(),
        });
    }
    let magic: [u8; 4] = bytes[0..4].try_into().expect("长度已检查");
    let kind = PortableObjectKind::from_magic(magic).ok_or(InspectError::UnknownMagic {
        magic: magic.to_vec(),
    })?;
    without_digest_probe(|| laneflow_format::preflight_object_registry(bytes, kind, format_limits))
        .map_err(InspectError::Preflight)
}

fn build_report<'a>(
    view: RegistryCheckedObjectView<'a>,
    options: &InspectOptions,
    report_limits: &ReportLimits,
    format_limits: FormatLimits,
    expand_rows_budget: &mut usize,
) -> InspectReport {
    let kind = view.kind();
    let no_directory = kind == PortableObjectKind::CanonicalPublicationDescriptor;
    let mut sections = Vec::with_capacity(usize::try_from(view.section_count()).unwrap_or(0));
    let mut fields = Vec::new();
    let mut truncated_tables = Vec::new();

    for (ordinal, section) in view.sections().enumerate() {
        let ordinal = u32::try_from(ordinal).unwrap_or(u32::MAX);
        let identity = section_identity(kind, &section, ordinal);
        if !section_selected(options, &identity) {
            continue;
        }
        let mut table_summaries = Vec::new();
        for (table_ordinal, table) in section.tables().enumerate() {
            let table_ordinal = u32::try_from(table_ordinal).unwrap_or(u32::MAX);
            let table_identity = table_identity(kind, ordinal, &table, table_ordinal);
            if !table_selected(options, &table_identity) {
                continue;
            }
            let chunks = (0..table.chunk_count())
                .filter_map(|chunk| {
                    Some(ChunkSummary {
                        index: table.chunk_index(chunk)?,
                        first_logical_row: table.chunk_first_logical_row(chunk)?,
                        row_count: table.chunk_row_count(chunk)?,
                        byte_offset: table.chunk_byte_offset(chunk)?,
                        byte_length: table.chunk_exact_byte_length(chunk)?,
                        digest: table.chunk_digest(chunk),
                    })
                })
                .collect();
            table_summaries.push(TableSummary {
                identity: table_identity,
                row_count: table.row_count(),
                chunk_count: table.chunk_count(),
                no_chunk_directory: no_directory,
                chunks,
            });

            if options.expand.fields {
                let Some(schema) = laneflow_static_contract::portable_object_schema(kind)
                    .sections
                    .get(usize::try_from(ordinal).unwrap_or(usize::MAX))
                    .and_then(|section| {
                        section
                            .tables
                            .get(usize::try_from(table_ordinal).unwrap_or(usize::MAX))
                    })
                else {
                    continue;
                };
                let range = effective_row_range(options, table.row_count());
                let rows_in_range = range.end.saturating_sub(range.start);
                let member_table = is_policy_local_change_table(kind, ordinal, table_ordinal);
                let shown = expand_table_fields(
                    view.bytes(),
                    &table,
                    &identity,
                    &table_identity,
                    range,
                    schema.row,
                    member_table,
                    report_limits,
                    format_limits,
                    expand_rows_budget,
                    &mut fields,
                );
                if shown < rows_in_range {
                    truncated_tables.push((table_identity, rows_in_range - shown));
                }
            }
        }
        sections.push(SectionSummary {
            identity,
            format_version: section.format_version(),
            byte_offset: section.byte_offset(),
            byte_length: section.byte_length(),
            table_count: section.table_count(),
            row_count: table_summaries
                .iter()
                .map(|table| u64::from(table.row_count))
                .sum(),
            tables: table_summaries,
        });
    }

    InspectReport {
        kind,
        format_version: view.format_version(),
        object_byte_length: u64::try_from(view.bytes().len()).unwrap_or(u64::MAX),
        sections,
        fields,
        truncated_tables,
    }
}

fn effective_row_range(options: &InspectOptions, row_count: u32) -> RowRange {
    match options.filters.rows {
        Some(range) => RowRange {
            start: range.start.min(row_count),
            end: range.end.min(row_count),
        },
        None => RowRange {
            start: 0,
            end: row_count,
        },
    }
}

fn section_selected(options: &InspectOptions, identity: &SectionIdentity) -> bool {
    match &options.filters.section {
        None => true,
        Some(SectionSelector::Ordinal(ordinal)) => *ordinal == identity.ordinal,
        Some(SectionSelector::Name(name)) => *name == identity.name,
    }
}

fn table_selected(options: &InspectOptions, identity: &TableIdentity) -> bool {
    match &options.filters.table {
        None => true,
        Some(TableSelector::Kind(kind)) => *kind == identity.kind,
        Some(TableSelector::Name(name)) => *name == identity.name,
    }
}

#[allow(clippy::too_many_arguments)]
fn expand_table_fields(
    object_bytes: &[u8],
    table: &laneflow_format::RegistryCheckedTableView<'_>,
    section: &SectionIdentity,
    table_identity: &TableIdentity,
    range: RowRange,
    row_schema: &laneflow_static_contract::PortableRowSchema,
    member_table: bool,
    report_limits: &ReportLimits,
    format_limits: FormatLimits,
    expand_rows_budget: &mut usize,
    fields: &mut Vec<FieldEntry>,
) -> u32 {
    let mut shown = 0_u32;
    for logical_row in range.start..range.end {
        if *expand_rows_budget == 0 {
            break;
        }
        let Some(row) = table.row(logical_row) else {
            break;
        };
        *expand_rows_budget -= 1;
        shown += 1;
        let member_kind = member_table.then(|| policy_member_kind(row)).flatten();
        for field in row.fields() {
            let location = FieldLocation::top_level(
                section,
                table_identity,
                logical_row,
                field.tag(),
                field_name(row_schema, field.tag()).unwrap_or(""),
                Some(object_offset(object_bytes, field.value_bytes())),
            );
            push_field_or_member(
                field,
                &location,
                member_kind,
                report_limits,
                format_limits,
                expand_rows_budget,
                fields,
            );
        }
    }
    shown
}

#[allow(clippy::too_many_arguments)]
fn push_field_or_member(
    field: laneflow_format::RegistryCheckedFieldView<'_>,
    location: &FieldLocation,
    member_kind: Option<u8>,
    report_limits: &ReportLimits,
    format_limits: FormatLimits,
    expand_rows_budget: &mut usize,
    fields: &mut Vec<FieldEntry>,
) {
    // LFSD 已登记成员 Bytes：只有明确登记为 RowV1 且已受检的 Bytes 才展开；
    // 预检失败的成员值退回不透明预览，不凭内容猜类型。
    if let Some(kind) = member_kind
        && matches!(
            field.field_type(),
            laneflow_static_contract::PortableFieldType::Bytes
        )
        && matches!(field.tag(), 5 | 6)
    {
        let member = embedded_row_display(
            kind,
            field.value_bytes(),
            report_limits,
            expand_rows_budget,
            format_limits,
        );
        if let Ok(display) = member {
            let mut member_location = location.clone();
            member_location
                .nested
                .push(crate::walk::NestedStep::PolicyMember {
                    kind,
                    field_tag: field.tag(),
                    field_name: location.field_name,
                });
            fields.push(FieldEntry {
                location: member_location,
                value: display,
            });
            return;
        }
    }
    if let Ok(value) = display_field_value(field, location, report_limits, expand_rows_budget) {
        fields.push(FieldEntry {
            location: location.clone(),
            value,
        });
    }
}
