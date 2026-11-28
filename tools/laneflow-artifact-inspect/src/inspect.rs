//! 单对象检视：识别、预检、事实域摘要与按登记展开。
//!
//! 分层：[`ObjectFacts`] 是 wire 真相（magic、版本、全对象节/表/行/chunk
//! 总数），在筛选作用之前计算、不可被展示策略改动；`sections` / `fields`
//! 是 [`DisplayPolicy`](crate::session::DisplayPolicy) 筛选后的展示明细。
//! 检查语义完全复用 `laneflow-format::preflight_object_values`（registry 结构
//! 与直接值域两层）：预检在无摘要探针的闭包内执行，本模块不解释未受检内容。

use laneflow_format::{FormatError, FormatLimits, RegistryCheckedObjectView};
use laneflow_static_contract::{PortableObjectKind, Sha256Digest};

use crate::session::{DisplayPolicy, InspectFilters, SessionConfig};
use crate::value::{RowBudget, ValueDisplay, display_field_value, embedded_row_display};
use crate::walk::{
    FieldPath, SectionIdentity, SideWalk, TableIdentity, field_name, is_policy_local_change_table,
    policy_member_kind, section_identity, table_identity,
};

/// 检视展开选项（属于展示策略的「展开维度」，与筛选互补）。
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

/// 检视选项。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InspectOptions {
    /// 展开集。
    pub expand: ExpandSet,
}

/// 全对象 wire 事实：在筛选作用之前计算，展示策略无路径可改写。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectFacts {
    /// 已识别并通过预检的对象种类。
    pub kind: PortableObjectKind,
    /// 对象格式版本。
    pub format_version: u16,
    /// 对象 exact bytes 长度。
    pub object_byte_length: u64,
    /// 精确节数。
    pub section_count: u32,
    /// 逻辑表总数。
    pub table_count: u32,
    /// 全对象总行数。
    pub row_count: u64,
    /// 物理 chunk 总数；LFCP 无 chunk 目录时为 `None`。
    pub chunk_count: Option<u32>,
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
    /// 字段逻辑路径。
    pub path: FieldPath,
    /// 本侧物理偏移。
    pub offset: u64,
    /// 值展示。
    pub value: ValueDisplay,
}

/// 检视报告：wire 事实 + 筛选后的展示明细。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InspectReport {
    /// 全对象事实（不可筛）。
    pub facts: ObjectFacts,
    /// 筛选后的节摘要。
    pub sections: Vec<SectionSummary>,
    /// 展开的字段条目（受筛选与限额约束）。
    pub fields: Vec<FieldEntry>,
    /// 字段展开因行数限额被截断的表与剩余行数。
    pub truncated_tables: Vec<(TableIdentity, u32)>,
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
        /// 前 4 个字节。
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

/// 对完整对象执行检视：识别、预检、事实摘要与（可选）字段展开。
///
/// 预检在 `without_digest_probe` 内执行，损坏输入不写 #794 取证转储。
/// 展示限额只影响 `fields` 展开条数；`facts` 与检查结论不受影响。
///
/// # Errors
///
/// 输入无法识别为当前支持的对象种类，或正式预检失败时返回 [`InspectError`]。
pub fn inspect(
    bytes: &[u8],
    options: &InspectOptions,
    policy: &DisplayPolicy,
    config: &SessionConfig,
) -> Result<InspectReport, InspectError> {
    let view = preflight_view(bytes, config.format)?;
    let facts = collect_facts(view);
    let mut remaining_rows = policy.limits.max_expanded_rows;
    let mut rows = RowBudget::new(&mut remaining_rows);
    let walk = SideWalk::new(view.bytes());
    let (sections, fields, truncated_tables) =
        build_selection(view, options, policy, config, &walk, &mut rows);
    Ok(InspectReport {
        facts,
        sections,
        fields,
        truncated_tables,
    })
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
    crate::check::preflight_object(bytes, kind, format_limits).map_err(InspectError::Preflight)
}

/// 事实域：遍历全对象（不受筛选影响）计算 wire 真相。
fn collect_facts(view: RegistryCheckedObjectView<'_>) -> ObjectFacts {
    let kind = view.kind();
    let no_directory = kind == PortableObjectKind::CanonicalPublicationDescriptor;
    let mut table_count = 0_u32;
    let mut row_count = 0_u64;
    let mut chunk_count = 0_u32;
    for section in view.sections() {
        for table in section.tables() {
            table_count += 1;
            row_count += u64::from(table.row_count());
            chunk_count += table.chunk_count();
        }
    }
    ObjectFacts {
        kind,
        format_version: view.format_version(),
        object_byte_length: u64::try_from(view.bytes().len()).unwrap_or(u64::MAX),
        section_count: view.section_count(),
        table_count,
        row_count,
        chunk_count: (!no_directory).then_some(chunk_count),
    }
}

/// 展示域：按策略筛选构建节/表/chunk 摘要与字段展开。
fn build_selection<'a>(
    view: RegistryCheckedObjectView<'a>,
    options: &InspectOptions,
    policy: &DisplayPolicy,
    config: &SessionConfig,
    walk: &SideWalk<'a>,
    rows: &mut RowBudget<'_>,
) -> (
    Vec<SectionSummary>,
    Vec<FieldEntry>,
    Vec<(TableIdentity, u32)>,
) {
    let kind = view.kind();
    let no_directory = kind == PortableObjectKind::CanonicalPublicationDescriptor;
    let mut sections = Vec::new();
    let mut fields = Vec::new();
    let mut truncated_tables = Vec::new();

    for (ordinal, section) in view.sections().enumerate() {
        let ordinal = u32::try_from(ordinal).unwrap_or(u32::MAX);
        let identity = section_identity(kind, &section, ordinal);
        if !policy
            .filters
            .selects_section(identity.ordinal, identity.name)
        {
            continue;
        }
        let mut table_summaries = Vec::new();
        for (table_ordinal, table) in section.tables().enumerate() {
            let table_ordinal = u32::try_from(table_ordinal).unwrap_or(u32::MAX);
            let table_identity = table_identity(kind, ordinal, &table, table_ordinal);
            if !policy
                .filters
                .selects_table(table_identity.kind, table_identity.name)
            {
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
                    .and_then(|section_schema| {
                        section_schema
                            .tables
                            .get(usize::try_from(table_ordinal).unwrap_or(usize::MAX))
                    })
                else {
                    continue;
                };
                let rows_in_range = count_rows_in_range(table.row_count(), &policy.filters);
                let shown = expand_table_fields(
                    walk,
                    &table,
                    &identity,
                    &table_identity,
                    policy,
                    schema.row,
                    is_policy_local_change_table(kind, ordinal, table_ordinal),
                    config,
                    rows,
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
    (sections, fields, truncated_tables)
}

/// 行范围内（筛选后的）应展开行数：用于与实际展开数对账截断。
fn count_rows_in_range(row_count: u32, filters: &InspectFilters) -> u32 {
    u32::try_from(
        (0..row_count)
            .filter(|row| filters.selects_row(*row))
            .count(),
    )
    .unwrap_or(u32::MAX)
}

#[allow(clippy::too_many_arguments)]
fn expand_table_fields(
    walk: &SideWalk<'_>,
    table: &laneflow_format::RegistryCheckedTableView<'_>,
    section: &SectionIdentity,
    table_identity: &TableIdentity,
    policy: &DisplayPolicy,
    row_schema: &laneflow_static_contract::PortableRowSchema,
    member_table: bool,
    config: &SessionConfig,
    rows: &mut RowBudget<'_>,
    fields: &mut Vec<FieldEntry>,
) -> u32 {
    let mut shown = 0_u32;
    for (logical_row, row) in walk.table_rows(*table) {
        if !policy.filters.selects_row(logical_row) {
            continue;
        }
        if !rows.try_consume() {
            break;
        }
        shown += 1;
        let member_kind = member_table.then(|| policy_member_kind(row)).flatten();
        for field in row.fields() {
            let path = FieldPath {
                section: *section,
                table: *table_identity,
                logical_row,
                field_tag: field.tag(),
                field_name: field_name(row_schema, field.tag()).unwrap_or(""),
                nested: Vec::new(),
            };
            push_field_or_member(
                field,
                &path,
                member_kind,
                walk,
                policy,
                config,
                rows,
                fields,
            );
        }
    }
    shown
}

#[allow(clippy::too_many_arguments)]
fn push_field_or_member(
    field: laneflow_format::RegistryCheckedFieldView<'_>,
    path: &FieldPath,
    member_kind: Option<u8>,
    walk: &SideWalk<'_>,
    policy: &DisplayPolicy,
    config: &SessionConfig,
    rows: &mut RowBudget<'_>,
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
            path,
            &policy.limits,
            rows,
            config.format,
        );
        if let Ok(display) = member {
            fields.push(FieldEntry {
                path: path.with_step(crate::walk::NestedStep::PolicyMember {
                    kind,
                    field_tag: field.tag(),
                    field_name: path.field_name,
                }),
                offset: walk.value_offset(field),
                value: display,
            });
            return;
        }
    }
    if let Ok(value) = display_field_value(field, path, &policy.limits, rows) {
        fields.push(FieldEntry {
            path: path.clone(),
            offset: walk.value_offset(field),
            value,
        });
    }
}
