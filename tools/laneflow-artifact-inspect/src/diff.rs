//! 结构化差异：完整比较两份对象，报告原始字节、容器与字段三类差异。
//!
//! 架构契约：
//! - **联接键与两侧观察分离**：[`FieldDiff`] 只携带逻辑 [`FieldPath`]（联接
//!   键）与两侧各自的 [`FieldSide`]（存在性、值展示、本侧物理偏移）——
//!   「共用一个偏移」「借用另一侧的 tag」在模型上不可能。
//! - **双通道收集**：事实通道（精确计数、逐表首差）不受展示策略影响且完整
//!   收集；展示通道（字段/容器条目）经 [`DisplayPolicy`] 筛选并受条数上限
//!   约束。比较本身始终遍历全对象，筛选不改变结论。
//! - **顺序遍历**：行/字段只经单游标迭代器访问，两侧各持一个
//!   [`SideWalk`]，偏移由本侧指针差给出。
//! - **三态摘要**：chunk 摘要比较是「一致 / 不同 / 不适用」，LFCP singleton
//!   无 chunk 目录不被误报为不同。

use laneflow_format::{
    FormatError, FormatLimits, RegistryCheckedFieldValue, RegistryCheckedFieldView,
    RegistryCheckedObjectView, RegistryCheckedRowView, RegistryCheckedTableView,
    checked_policy_member_row, preflight_object_registry, without_digest_probe,
};
use laneflow_static_contract::{
    PortableFieldType, PortableObjectKind, PortableRowSchema, PortableRowShape, Sha256Digest,
};

use crate::session::{DisplayPolicy, SessionConfig};
use crate::value::{RowBudget, ValueDisplay, bytes_preview, display_field_value};
use crate::walk::{
    FieldPath, NestedStep, SectionIdentity, SideWalk, TableIdentity, field_name,
    is_policy_local_change_table, policy_member_kind, section_identity, table_identity,
};

/// 单侧检查结果。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SideOutcome {
    /// 该侧读入的对象字节数。
    pub length: u64,
    /// 预检结果：对象种类或 [`FormatError`]。
    pub check: Result<PortableObjectKind, FormatError>,
}

/// 原始字节差异（解码后对象 bytes）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ByteDivergence {
    /// 首个分歧的绝对偏移与两侧字节值。
    Offset {
        /// 分歧偏移。
        offset: u64,
        /// expected 侧字节。
        expected: u8,
        /// actual 侧字节。
        actual: u8,
    },
    /// 前缀相同：短侧结束位置与长侧。
    Eof {
        /// 短侧结束偏移（即相等前缀长度）。
        offset: u64,
        /// 较长的一侧。
        longer_side: Side,
    },
}

/// 比较方向侧。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Side {
    /// expected/old（左）。
    Expected,
    /// actual/new（右）。
    Actual,
}

/// 容器差异类别。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContainerKind {
    /// 对象 exact bytes 长度。
    ObjectLength,
    /// 节相对对象起点的偏移。
    SectionOffset(SectionIdentity),
    /// 节 exact 长度。
    SectionLength(SectionIdentity),
    /// 表的物理 chunk 数。
    TableChunkCount(SectionIdentity, TableIdentity),
    /// chunk 相对节起点的偏移。
    ChunkOffset(SectionIdentity, TableIdentity, u32),
    /// chunk exact 长度。
    ChunkLength(SectionIdentity, TableIdentity, u32),
    /// chunk 目录摘要。
    ChunkDigest(SectionIdentity, TableIdentity, u32),
}

/// 一条容器差异。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContainerDiff {
    /// 类别。
    pub kind: ContainerKind,
    /// expected/old 侧值。
    pub expected: u64,
    /// actual/new 侧值。
    pub actual: u64,
    /// 目录摘要差异时的两侧摘要。
    pub digests: Option<(Sha256Digest, Sha256Digest)>,
}

impl ContainerDiff {
    /// 节身份（对象级差异返回 `None`）。
    #[must_use]
    pub fn section(&self) -> Option<&SectionIdentity> {
        match &self.kind {
            ContainerKind::ObjectLength => None,
            ContainerKind::SectionOffset(section)
            | ContainerKind::SectionLength(section)
            | ContainerKind::TableChunkCount(section, _)
            | ContainerKind::ChunkOffset(section, _, _)
            | ContainerKind::ChunkLength(section, _, _)
            | ContainerKind::ChunkDigest(section, _, _) => Some(section),
        }
    }

    /// 表身份（节级及以上差异返回 `None`）。
    #[must_use]
    pub fn table(&self) -> Option<&TableIdentity> {
        match &self.kind {
            ContainerKind::TableChunkCount(_, table)
            | ContainerKind::ChunkOffset(_, table, _)
            | ContainerKind::ChunkLength(_, table, _)
            | ContainerKind::ChunkDigest(_, table, _) => Some(table),
            _ => None,
        }
    }
}

/// 字段差异的一侧观察。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FieldSide {
    /// 该侧缺失该字段（与零值、空字符串区分）。
    Missing,
    /// 该侧存在该字段。
    Present {
        /// 值展示。
        value: ValueDisplay,
        /// 值载荷在本侧对象 exact bytes 内的绝对偏移。
        offset: u64,
    },
}

/// 一条字段差异（叶级，含完整嵌套路径与两侧各自观察）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldDiff {
    /// 逻辑联接键。
    pub path: FieldPath,
    /// expected/old 侧观察。
    pub expected: FieldSide,
    /// actual/new 侧观察。
    pub actual: FieldSide,
}

/// 行数差异。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RowCountDiff {
    /// 节身份。
    pub section: SectionIdentity,
    /// 表身份。
    pub table: TableIdentity,
    /// expected/old 侧行数。
    pub expected: u32,
    /// actual/new 侧行数。
    pub actual: u32,
}

/// chunk 目录摘要的三态比较。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DigestComparison {
    /// LFCP singleton：没有 chunk 目录，不存在 chunkDigest。
    NotApplicable,
    /// 两侧全部 chunk 摘要一致。
    Equal,
    /// 摘要不同：首个分歧 chunk 的两侧摘要（仅 chunk 数不同时为 `None`）。
    Differ {
        /// expected 侧摘要。
        expected: Option<Sha256Digest>,
        /// actual 侧摘要。
        actual: Option<Sha256Digest>,
    },
}

/// 一张表的逐表汇总（事实通道：完整收集，不受展示策略影响）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableDiffSummary {
    /// 节身份。
    pub section: SectionIdentity,
    /// 表身份。
    pub table: TableIdentity,
    /// 行数（两侧）。
    pub row_counts: (u32, u32),
    /// 两侧 chunk 目录摘要比较。
    pub chunk_digests: DigestComparison,
    /// 该表首个字段差异（存在时；事实通道完整收集）。
    pub first_field_diff: Option<FieldDiff>,
}

/// 完整差异报告。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffReport {
    /// expected/old（左）侧检查结果。
    pub expected: SideOutcome,
    /// actual/new（右）侧检查结果。
    pub actual: SideOutcome,
    /// 解码后对象 bytes 是否精确相等。
    pub bytes_equal: bool,
    /// 首个原始字节分歧（bytes_equal 时为 `None`）。
    pub byte_divergence: Option<ByteDivergence>,
    /// 容器差异（展示通道：筛选感知、受条数上限约束）。
    pub container: Vec<ContainerDiff>,
    /// 容器差异总数（事实：精确计数）。
    pub container_diff_total: usize,
    /// 字段差异（展示通道：筛选感知、受条数上限约束）。
    pub fields: Vec<FieldDiff>,
    /// 字段差异总数（事实：精确计数，不受筛选影响）。
    pub field_diff_total: usize,
    /// 行数差异（事实通道：完整收集，≤ 登记表数）。
    pub row_counts: Vec<RowCountDiff>,
    /// 逐表汇总（事实通道：完整收集）。
    pub tables: Vec<TableDiffSummary>,
    /// 两侧预检均通过且种类一致。
    pub both_checked_same_kind: bool,
}

impl DiffReport {
    /// 判定结论是否为「无法完成比较」（任一侧预检失败或种类不一致）。
    #[must_use]
    pub fn incomplete(&self) -> bool {
        !self.both_checked_same_kind
    }

    /// 完整结论：bytes 相等且两侧检查通过。
    #[must_use]
    pub fn equal(&self) -> bool {
        self.both_checked_same_kind && self.bytes_equal
    }
}

/// 对两份完整读入的对象 bytes 执行差异比较。
///
/// 预检使用 `config.format`（只能收紧构造）；展示条目经 `policy` 筛选，
/// 事实通道（计数、逐表首差）不受筛选影响。任一侧预检失败时报告两侧
/// 检查状态并保留原始字节差异，不做容器与字段比较。
#[must_use]
pub fn diff_bytes(
    expected: &[u8],
    actual: &[u8],
    policy: &DisplayPolicy,
    config: &SessionConfig,
) -> DiffReport {
    let expected_view = checked_view(expected, config.format);
    let actual_view = checked_view(actual, config.format);
    let byte_divergence = first_divergence(expected, actual);

    let mut report = DiffReport {
        expected: SideOutcome {
            length: u64::try_from(expected.len()).unwrap_or(u64::MAX),
            check: expected_view
                .as_ref()
                .map(|view| view.kind())
                .map_err(|error| *error),
        },
        actual: SideOutcome {
            length: u64::try_from(actual.len()).unwrap_or(u64::MAX),
            check: actual_view
                .as_ref()
                .map(|view| view.kind())
                .map_err(|error| *error),
        },
        bytes_equal: expected == actual,
        byte_divergence,
        container: Vec::new(),
        container_diff_total: 0,
        fields: Vec::new(),
        field_diff_total: 0,
        row_counts: Vec::new(),
        tables: Vec::new(),
        both_checked_same_kind: false,
    };

    let (Ok(expected_view), Ok(actual_view)) = (&expected_view, &actual_view) else {
        return report;
    };
    if expected_view.kind() != actual_view.kind() {
        return report;
    }
    report.both_checked_same_kind = true;
    if report.bytes_equal {
        return report;
    }

    let mut collector = DiffCollector {
        walk_e: SideWalk::new(expected_view.bytes()),
        walk_a: SideWalk::new(actual_view.bytes()),
        policy,
        config,
        container: Vec::new(),
        container_total: 0,
        fields: Vec::new(),
        field_total: 0,
        row_counts: Vec::new(),
        tables: Vec::new(),
    };
    compare_structures(expected_view, actual_view, &mut collector);
    report.container = collector.container;
    report.container_diff_total = collector.container_total;
    report.fields = collector.fields;
    report.field_diff_total = collector.field_total;
    report.row_counts = collector.row_counts;
    report.tables = collector.tables;
    report
}

fn checked_view(
    bytes: &[u8],
    format_limits: FormatLimits,
) -> Result<RegistryCheckedObjectView<'_>, FormatError> {
    if bytes.len() < 32 {
        return Err(FormatError::Truncated {
            structure: laneflow_format::FormatStructure::ObjectPreamble,
            offset: 32,
            needed: 32,
            available: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        });
    }
    let magic: [u8; 4] = bytes[0..4].try_into().expect("长度已检查");
    let Some(kind) = PortableObjectKind::from_magic(magic) else {
        return Err(FormatError::UnknownKind {
            structure: laneflow_format::FormatStructure::ObjectPreamble,
            code: u64::from(u32::from_le_bytes([magic[0], magic[1], magic[2], magic[3]])),
        });
    };
    without_digest_probe(|| preflight_object_registry(bytes, kind, format_limits))
}

fn first_divergence(expected: &[u8], actual: &[u8]) -> Option<ByteDivergence> {
    let common = expected.len().min(actual.len());
    for (offset, (left, right)) in expected[..common].iter().zip(&actual[..common]).enumerate() {
        if left != right {
            return Some(ByteDivergence::Offset {
                offset: u64::try_from(offset).unwrap_or(u64::MAX),
                expected: *left,
                actual: *right,
            });
        }
    }
    if expected.len() == actual.len() {
        return None;
    }
    Some(ByteDivergence::Eof {
        offset: u64::try_from(common).unwrap_or(u64::MAX),
        longer_side: if expected.len() > actual.len() {
            Side::Expected
        } else {
            Side::Actual
        },
    })
}

/// 行比较上下文：一行的逻辑位置（联接键基底）+ 当前表在事实通道中的序号。
struct RowScope {
    section: SectionIdentity,
    table: TableIdentity,
    logical_row: u32,
    nested: Vec<NestedStep>,
    table_index: usize,
}

impl RowScope {
    fn leaf_path(&self, tag: u16, name: &'static str) -> FieldPath {
        FieldPath {
            section: self.section,
            table: self.table,
            logical_row: self.logical_row,
            field_tag: tag,
            field_name: name,
            nested: self.nested.clone(),
        }
    }

    fn nested_scope(&self, step: NestedStep) -> RowScope {
        let mut nested = self.clone();
        nested.nested.push(step);
        nested
    }
}

impl Clone for RowScope {
    fn clone(&self) -> Self {
        Self {
            section: self.section,
            table: self.table,
            logical_row: self.logical_row,
            nested: self.nested.clone(),
            table_index: self.table_index,
        }
    }
}

/// 双通道收集器。
struct DiffCollector<'a> {
    walk_e: SideWalk<'a>,
    walk_a: SideWalk<'a>,
    policy: &'a DisplayPolicy,
    config: &'a SessionConfig,
    container: Vec<ContainerDiff>,
    container_total: usize,
    fields: Vec<FieldDiff>,
    field_total: usize,
    row_counts: Vec<RowCountDiff>,
    tables: Vec<TableDiffSummary>,
}

impl DiffCollector<'_> {
    /// 事实通道：精确计数与逐表首差；展示通道：筛选感知 + 条数上限。
    fn push_field(&mut self, diff: FieldDiff, table_index: usize) {
        self.field_total += 1;
        let table = &mut self.tables[table_index];
        if table.first_field_diff.is_none() {
            table.first_field_diff = Some(diff.clone());
        }
        if self.fields.len() < self.policy.limits.max_diff_entries
            && self.policy.selects_field_path(
                diff.path.section.ordinal,
                diff.path.section.name,
                diff.path.table.kind,
                diff.path.table.name,
                diff.path.logical_row,
            )
        {
            self.fields.push(diff);
        }
    }

    fn push_container(&mut self, diff: ContainerDiff) {
        self.container_total += 1;
        let selected = match (diff.section(), diff.table()) {
            (None, None) => true,
            (Some(section), None) => self
                .policy
                .filters
                .selects_section(section.ordinal, section.name),
            (Some(section), Some(table)) => {
                self.policy
                    .filters
                    .selects_section(section.ordinal, section.name)
                    && self.policy.filters.selects_table(table.kind, table.name)
            }
            (None, Some(_)) => false,
        };
        if self.container.len() < self.policy.limits.max_diff_entries && selected {
            self.container.push(diff);
        }
    }
}

fn compare_structures(
    expected: &RegistryCheckedObjectView<'_>,
    actual: &RegistryCheckedObjectView<'_>,
    collector: &mut DiffCollector<'_>,
) {
    if expected.bytes().len() != actual.bytes().len() {
        collector.push_container(ContainerDiff {
            kind: ContainerKind::ObjectLength,
            expected: u64::try_from(expected.bytes().len()).unwrap_or(u64::MAX),
            actual: u64::try_from(actual.bytes().len()).unwrap_or(u64::MAX),
            digests: None,
        });
    }
    let kind = expected.kind();
    let no_directory = kind == PortableObjectKind::CanonicalPublicationDescriptor;
    for (ordinal, (expected_section, actual_section)) in
        expected.sections().zip(actual.sections()).enumerate()
    {
        let ordinal = u32::try_from(ordinal).unwrap_or(u32::MAX);
        let section_id = section_identity(kind, &expected_section, ordinal);
        if expected_section.byte_offset() != actual_section.byte_offset() {
            collector.push_container(ContainerDiff {
                kind: ContainerKind::SectionOffset(section_id),
                expected: expected_section.byte_offset(),
                actual: actual_section.byte_offset(),
                digests: None,
            });
        }
        if expected_section.byte_length() != actual_section.byte_length() {
            collector.push_container(ContainerDiff {
                kind: ContainerKind::SectionLength(section_id),
                expected: expected_section.byte_length(),
                actual: actual_section.byte_length(),
                digests: None,
            });
        }
        for (table_ordinal, (expected_table, actual_table)) in expected_section
            .tables()
            .zip(actual_section.tables())
            .enumerate()
        {
            let table_ordinal = u32::try_from(table_ordinal).unwrap_or(u32::MAX);
            let table_id = table_identity(kind, ordinal, &expected_table, table_ordinal);
            let table_index = collector.tables.len();
            collector.tables.push(TableDiffSummary {
                section: section_id,
                table: table_id,
                row_counts: (expected_table.row_count(), actual_table.row_count()),
                chunk_digests: digest_comparison(&expected_table, &actual_table, no_directory),
                first_field_diff: None,
            });
            compare_table(
                kind,
                ordinal,
                table_ordinal,
                section_id,
                table_id,
                table_index,
                &expected_table,
                &actual_table,
                collector,
            );
        }
    }
}

/// chunk 目录摘要三态比较。
fn digest_comparison(
    expected: &RegistryCheckedTableView<'_>,
    actual: &RegistryCheckedTableView<'_>,
    no_directory: bool,
) -> DigestComparison {
    if no_directory {
        return DigestComparison::NotApplicable;
    }
    let common = expected.chunk_count().min(actual.chunk_count());
    for chunk in 0..common {
        let expected_digest = expected.chunk_digest(chunk);
        let actual_digest = actual.chunk_digest(chunk);
        if expected_digest != actual_digest {
            return DigestComparison::Differ {
                expected: expected_digest,
                actual: actual_digest,
            };
        }
    }
    if expected.chunk_count() == actual.chunk_count() {
        DigestComparison::Equal
    } else {
        DigestComparison::Differ {
            expected: None,
            actual: None,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn compare_table(
    kind: PortableObjectKind,
    section_ordinal: u32,
    table_ordinal: u32,
    section: SectionIdentity,
    table: TableIdentity,
    table_index: usize,
    expected_table: &RegistryCheckedTableView<'_>,
    actual_table: &RegistryCheckedTableView<'_>,
    collector: &mut DiffCollector<'_>,
) {
    if expected_table.chunk_count() != actual_table.chunk_count() {
        collector.push_container(ContainerDiff {
            kind: ContainerKind::TableChunkCount(section, table),
            expected: u64::from(expected_table.chunk_count()),
            actual: u64::from(actual_table.chunk_count()),
            digests: None,
        });
    }
    for chunk in 0..expected_table.chunk_count().min(actual_table.chunk_count()) {
        if expected_table.chunk_byte_offset(chunk) != actual_table.chunk_byte_offset(chunk) {
            collector.push_container(ContainerDiff {
                kind: ContainerKind::ChunkOffset(section, table, chunk),
                expected: expected_table.chunk_byte_offset(chunk).unwrap_or(0),
                actual: actual_table.chunk_byte_offset(chunk).unwrap_or(0),
                digests: None,
            });
        }
        if expected_table.chunk_exact_byte_length(chunk)
            != actual_table.chunk_exact_byte_length(chunk)
        {
            collector.push_container(ContainerDiff {
                kind: ContainerKind::ChunkLength(section, table, chunk),
                expected: expected_table.chunk_exact_byte_length(chunk).unwrap_or(0),
                actual: actual_table.chunk_exact_byte_length(chunk).unwrap_or(0),
                digests: None,
            });
        }
        if expected_table.chunk_digest(chunk).is_some()
            && expected_table.chunk_digest(chunk) != actual_table.chunk_digest(chunk)
        {
            collector.push_container(ContainerDiff {
                kind: ContainerKind::ChunkDigest(section, table, chunk),
                expected: 0,
                actual: 0,
                digests: Some((
                    expected_table
                        .chunk_digest(chunk)
                        .unwrap_or(Sha256Digest::ZERO),
                    actual_table
                        .chunk_digest(chunk)
                        .unwrap_or(Sha256Digest::ZERO),
                )),
            });
        }
    }

    if expected_table.row_count() != actual_table.row_count() {
        collector.row_counts.push(RowCountDiff {
            section,
            table,
            expected: expected_table.row_count(),
            actual: actual_table.row_count(),
        });
    }

    let Some(schema) = laneflow_static_contract::portable_object_schema(kind)
        .sections
        .get(usize::try_from(section_ordinal).unwrap_or(usize::MAX))
        .and_then(|section_schema| {
            section_schema
                .tables
                .get(usize::try_from(table_ordinal).unwrap_or(usize::MAX))
        })
    else {
        return;
    };
    let member_table = is_policy_local_change_table(kind, section_ordinal, table_ordinal);

    // 顺序 zip 两侧单游标行迭代器：不做 row(i) 随机重扫。
    for ((logical_row, expected_row), (_, actual_row)) in collector
        .walk_e
        .table_rows(*expected_table)
        .zip(collector.walk_a.table_rows(*actual_table))
    {
        let scope = RowScope {
            section,
            table,
            logical_row,
            nested: Vec::new(),
            table_index,
        };
        compare_rows(
            &scope,
            schema.row,
            member_table,
            expected_row,
            actual_row,
            collector,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn compare_rows(
    scope: &RowScope,
    row_schema: &PortableRowSchema,
    member_table: bool,
    expected_row: RegistryCheckedRowView<'_>,
    actual_row: RegistryCheckedRowView<'_>,
    collector: &mut DiffCollector<'_>,
) {
    // LFSD 策略局部变更行：两侧成员 kind 一致时才按成员登记展开。
    let member_kind = if member_table {
        match (
            policy_member_kind(expected_row),
            policy_member_kind(actual_row),
        ) {
            (Some(left), Some(right)) if left == right => Some(left),
            _ => None,
        }
    } else {
        None
    };
    // 字段按 tag 合并比较：先决定两侧各自消费与否，再用「被消费字段自己的
    // tag」定位——不借用另一侧的标签。
    let mut expected_fields = expected_row.fields().peekable();
    let mut actual_fields = actual_row.fields().peekable();
    loop {
        let expected_tag = expected_fields.peek().map(|field| field.tag());
        let actual_tag = actual_fields.peek().map(|field| field.tag());
        let mut consume_expected = false;
        let mut consume_actual = false;
        let tag = match (expected_tag, actual_tag) {
            (None, None) => break,
            (Some(tag), Some(_)) if tag == actual_tag.expect("同侧已 peek") => {
                consume_expected = true;
                consume_actual = true;
                tag
            }
            (Some(tag), None) => {
                consume_expected = true;
                tag
            }
            (None, Some(tag)) => {
                consume_actual = true;
                tag
            }
            (Some(expected), Some(actual)) => {
                if expected < actual {
                    consume_expected = true;
                    expected
                } else {
                    consume_actual = true;
                    actual
                }
            }
        };
        let name = field_name(row_schema, tag).unwrap_or("");
        match (consume_expected, consume_actual) {
            (true, true) => {
                let expected_field = expected_fields.next().expect("peek 已确认");
                let actual_field = actual_fields.next().expect("peek 已确认");
                compare_values(
                    scope,
                    &scope.leaf_path(tag, name),
                    member_kind,
                    row_schema,
                    expected_field,
                    actual_field,
                    collector,
                );
            }
            (true, false) => {
                let expected_field = expected_fields.next().expect("peek 已确认");
                collector.push_field(
                    FieldDiff {
                        path: scope.leaf_path(tag, name),
                        expected: present_side(
                            &expected_field,
                            collector.walk_e,
                            &collector.policy.limits,
                        ),
                        actual: FieldSide::Missing,
                    },
                    scope.table_index,
                );
            }
            (false, true) => {
                let actual_field = actual_fields.next().expect("peek 已确认");
                collector.push_field(
                    FieldDiff {
                        path: scope.leaf_path(tag, name),
                        expected: FieldSide::Missing,
                        actual: present_side(
                            &actual_field,
                            collector.walk_a,
                            &collector.policy.limits,
                        ),
                    },
                    scope.table_index,
                );
            }
            (false, false) => unreachable!("合并决策至少消费一侧"),
        }
    }
}

fn present_side(
    field: &RegistryCheckedFieldView<'_>,
    walk: SideWalk<'_>,
    limits: &crate::session::ReportLimits,
) -> FieldSide {
    FieldSide::Present {
        value: preview_value(*field, limits),
        offset: walk.value_offset(*field),
    }
}

fn preview_value(
    field: RegistryCheckedFieldView<'_>,
    limits: &crate::session::ReportLimits,
) -> ValueDisplay {
    let mut remaining = 1_usize;
    let mut rows = RowBudget::new(&mut remaining);
    let path = FieldPath {
        section: SectionIdentity {
            ordinal: 0,
            name: "",
        },
        table: TableIdentity { kind: 0, name: "" },
        logical_row: 0,
        field_tag: field.tag(),
        field_name: "",
        nested: Vec::new(),
    };
    if let Ok(display) = display_field_value(field, &path, limits, &mut rows) {
        return display;
    }
    bytes_preview(field.value_bytes(), limits)
}

#[allow(clippy::too_many_arguments)]
fn compare_values(
    scope: &RowScope,
    path: &FieldPath,
    member_kind: Option<u8>,
    row_schema: &PortableRowSchema,
    expected_field: RegistryCheckedFieldView<'_>,
    actual_field: RegistryCheckedFieldView<'_>,
    collector: &mut DiffCollector<'_>,
) {
    // LFSD 已登记成员 Bytes：两侧按同一成员 kind 展开内嵌行，递归到叶。
    if let Some(kind) = member_kind
        && matches!(expected_field.field_type(), PortableFieldType::Bytes)
        && matches!(expected_field.tag(), 5 | 6)
    {
        let expected_member =
            checked_policy_member_row(kind, expected_field.value_bytes(), collector.config.format);
        let actual_member =
            checked_policy_member_row(kind, actual_field.value_bytes(), collector.config.format);
        if let (Ok(expected_row), Ok(actual_row)) = (expected_member, actual_member) {
            let schema = laneflow_static_contract::policy_local_value_schema(kind)
                .unwrap_or(&EMPTY_ROW_SCHEMA);
            let nested = scope.nested_scope(NestedStep::PolicyMember {
                kind,
                field_tag: expected_field.tag(),
                field_name: path.field_name,
            });
            compare_rows(&nested, schema, false, expected_row, actual_row, collector);
            return;
        }
    }

    // 嵌套 RecordVector：项数不同在本层报告；相同则逐项递归到叶。
    if let (
        Ok(RegistryCheckedFieldValue::RecordVector(expected_vector)),
        Ok(RegistryCheckedFieldValue::RecordVector(actual_vector)),
    ) = (expected_field.value(), actual_field.value())
        && expected_vector.len() == actual_vector.len()
    {
        let nested_schema = row_schema
            .fields
            .iter()
            .find(|field| field.tag == expected_field.tag())
            .and_then(|field| field.nested_row)
            .unwrap_or(&EMPTY_ROW_SCHEMA);
        let common = expected_vector.len();
        for (index, (expected_row, actual_row)) in expected_vector
            .rows()
            .zip(actual_vector.rows())
            .enumerate()
            .take(usize::try_from(common).unwrap_or(usize::MAX))
        {
            if !rows_fieldwise_equal(&expected_row, &actual_row) {
                let nested = scope.nested_scope(NestedStep::RecordVectorItem {
                    index: u32::try_from(index).unwrap_or(u32::MAX),
                    field_tag: path.field_tag,
                    field_name: path.field_name,
                });
                compare_rows(
                    &nested,
                    nested_schema,
                    false,
                    expected_row,
                    actual_row,
                    collector,
                );
            }
        }
        return;
    }

    if !values_equal(expected_field, actual_field) {
        collector.push_field(
            FieldDiff {
                path: path.clone(),
                expected: present_side(&expected_field, collector.walk_e, &collector.policy.limits),
                actual: present_side(&actual_field, collector.walk_a, &collector.policy.limits),
            },
            scope.table_index,
        );
    }
}

static EMPTY_ROW_SCHEMA: PortableRowSchema = PortableRowSchema {
    fields: &[],
    shape: PortableRowShape::Uniform,
};

fn values_equal(
    expected: RegistryCheckedFieldView<'_>,
    actual: RegistryCheckedFieldView<'_>,
) -> bool {
    let (Ok(expected_value), Ok(actual_value)) = (expected.value(), actual.value()) else {
        return expected.value_bytes() == actual.value_bytes();
    };
    match (expected_value, actual_value) {
        (RegistryCheckedFieldValue::U8(left), RegistryCheckedFieldValue::U8(right)) => {
            left == right
        }
        (RegistryCheckedFieldValue::U16(left), RegistryCheckedFieldValue::U16(right)) => {
            left == right
        }
        (RegistryCheckedFieldValue::U32(left), RegistryCheckedFieldValue::U32(right)) => {
            left == right
        }
        (RegistryCheckedFieldValue::U64(left), RegistryCheckedFieldValue::U64(right)) => {
            left == right
        }
        (RegistryCheckedFieldValue::I32(left), RegistryCheckedFieldValue::I32(right)) => {
            left == right
        }
        // 预检拒绝非规范值；位模式相等即值相等。
        (RegistryCheckedFieldValue::F32(left), RegistryCheckedFieldValue::F32(right)) => {
            left.to_bits() == right.to_bits()
        }
        (RegistryCheckedFieldValue::F64(left), RegistryCheckedFieldValue::F64(right)) => {
            left.to_bits() == right.to_bits()
        }
        (
            RegistryCheckedFieldValue::StableId128(left),
            RegistryCheckedFieldValue::StableId128(right),
        ) => left == right,
        (RegistryCheckedFieldValue::Sha256(left), RegistryCheckedFieldValue::Sha256(right)) => {
            left == right
        }
        (RegistryCheckedFieldValue::Utf8(left), RegistryCheckedFieldValue::Utf8(right)) => {
            left == right
        }
        (RegistryCheckedFieldValue::Bytes(left), RegistryCheckedFieldValue::Bytes(right)) => {
            left == right
        }
        (
            RegistryCheckedFieldValue::OrdinalVectorU32(left),
            RegistryCheckedFieldValue::OrdinalVectorU32(right),
        ) => {
            left.len() == right.len()
                && (0..left.len()).all(|index| left.get(index) == right.get(index))
        }
        (
            RegistryCheckedFieldValue::RecordVector(left),
            RegistryCheckedFieldValue::RecordVector(right),
        ) => {
            left.len() == right.len()
                && left
                    .rows()
                    .zip(right.rows())
                    .all(|(expected_row, actual_row)| {
                        rows_fieldwise_equal(&expected_row, &actual_row)
                    })
        }
        _ => false,
    }
}

fn rows_fieldwise_equal(
    expected: &RegistryCheckedRowView<'_>,
    actual: &RegistryCheckedRowView<'_>,
) -> bool {
    let mut expected_fields = expected.fields().peekable();
    let mut actual_fields = actual.fields().peekable();
    loop {
        let expected_tag = expected_fields.peek().map(|field| field.tag());
        let actual_tag = actual_fields.peek().map(|field| field.tag());
        match (expected_tag, actual_tag) {
            (None, None) => return true,
            (Some(left), Some(right)) if left == right => {
                let expected_field = expected_fields.next().expect("peek 已确认");
                let actual_field = actual_fields.next().expect("peek 已确认");
                if !values_equal(expected_field, actual_field) {
                    return false;
                }
            }
            _ => return false,
        }
    }
}

/// FormatError 的确定性展示：保留正式读取器的偏移语义（结构局部坐标或未知），
/// 不冒充文件绝对位置。
#[must_use]
pub fn format_error_display(error: &FormatError) -> String {
    format!("{error:?}")
}

/// 字节值的确定性展示。
#[must_use]
pub fn byte_hex(byte: u8) -> String {
    crate::value::hex_string(&[byte], 2)
}
