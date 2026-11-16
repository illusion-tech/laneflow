//! 结构化差异：完整比较两份对象，报告原始字节、容器与字段三类差异。
//!
//! 比较始终对全对象执行：相等判据是解码后对象 exact bytes 的精确相等，
//! 展示限额与筛选只影响渲染，不改变结论。字段差异按节、表、零基全表逻辑行、
//! 字段 tag 的顺序遍历，行按逻辑序号对齐（不做 StableId/LCS），字段按 tag
//! 合并比较；嵌套 RecordVector 与 LFSD 已登记内嵌行沿完整路径递归到叶字段。

use laneflow_format::{
    FormatError, FormatLimits, RegistryCheckedFieldValue, RegistryCheckedFieldView,
    RegistryCheckedObjectView, RegistryCheckedRowView, RegistryCheckedTableView,
    checked_policy_member_row, preflight_object_registry, without_digest_probe,
};
use laneflow_static_contract::{
    PortableFieldType, PortableObjectKind, PortableRowSchema, PortableRowShape, Sha256Digest,
};

use crate::budget::ReportLimits;
use crate::inspect::InspectFilters;
use crate::value::{ValueDisplay, bytes_preview, display_field_value, hex_string};
use crate::walk::{
    FieldLocation, NestedStep, SectionIdentity, TableIdentity, field_name,
    is_policy_local_change_table, object_offset, policy_member_kind, section_identity,
    table_identity,
};

/// 差异选项：筛选只影响渲染，不影响比较范围与结论。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DiffOptions {
    /// 展示筛选（节/表/行范围）。
    pub filters: InspectFilters,
}

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

/// 叶字段值的快照；`Missing` 表示字段缺失（与零值、空字符串区分）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValueSnapshot {
    /// 该侧缺失该字段。
    Missing,
    /// 该侧存在该字段。
    Present(ValueDisplay),
}

/// 一条字段差异（叶级，含完整嵌套路径）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldDiff {
    /// 字段路径与物理位置。
    pub location: FieldLocation,
    /// expected/old 侧值。
    pub expected: ValueSnapshot,
    /// actual/new 侧值。
    pub actual: ValueSnapshot,
}

/// 行数差异。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RowCountDiff {
    /// 表身份。
    pub identity: TableIdentity,
    /// expected/old 侧行数。
    pub expected: u32,
    /// actual/new 侧行数。
    pub actual: u32,
}

/// 一张变化表的逐表汇总。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableDiffSummary {
    /// 节身份。
    pub section: SectionIdentity,
    /// 表身份。
    pub table: TableIdentity,
    /// 行数（两侧）。
    pub row_counts: (u32, u32),
    /// 两侧 chunk 目录摘要是否全部一致。
    pub chunk_digests_equal: bool,
    /// 该表首个字段差异（存在时）。
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
    /// 容器差异（按节/表/chunk 顺序，受收集上限约束）。
    pub container: Vec<ContainerDiff>,
    /// 字段差异（按遍历顺序，受收集上限约束）。
    pub fields: Vec<FieldDiff>,
    /// 字段差异总数（精确计数，可大于 `fields.len()`）。
    pub field_diff_total: usize,
    /// 容器差异总数（精确计数）。
    pub container_diff_total: usize,
    /// 行数差异。
    pub row_counts: Vec<RowCountDiff>,
    /// 变化表汇总。
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

/// 行比较路径：节/表/逻辑行 + 嵌套前缀，构造叶字段完整路径。
#[derive(Clone, Debug)]
struct RowPath {
    section: SectionIdentity,
    table: TableIdentity,
    logical_row: u32,
    nested: Vec<NestedStep>,
}

impl RowPath {
    fn location(
        &self,
        field: Option<RegistryCheckedFieldView<'_>>,
        tag: u16,
        name: &'static str,
        object_bytes: &[u8],
    ) -> FieldLocation {
        FieldLocation {
            section_ordinal: self.section.ordinal,
            section_name: self.section.name,
            table_kind: self.table.kind,
            table_name: self.table.name,
            logical_row: self.logical_row,
            field_tag: tag,
            field_name: name,
            nested: self.nested.clone(),
            object_offset: field.map(|field| object_offset(object_bytes, field.value_bytes())),
        }
    }

    fn with_step(&self, step: NestedStep) -> Self {
        let mut nested = self.clone();
        nested.nested.push(step);
        nested
    }
}

/// 对两份完整读入的对象 bytes 执行差异比较。
///
/// 两侧预检在 `without_digest_probe` 内执行。任一侧失败时报告两侧检查状态并
/// 保留原始字节差异，不做容器与字段比较。
#[must_use]
pub fn diff_bytes(expected: &[u8], actual: &[u8]) -> DiffReport {
    let expected_view = checked_view(expected);
    let actual_view = checked_view(actual);
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
        fields: Vec::new(),
        field_diff_total: 0,
        container_diff_total: 0,
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

    let limits = ReportLimits::default();
    let mut collector = DiffCollector {
        limits,
        format_limits: FormatLimits::HARD,
        container: Vec::new(),
        container_total: 0,
        fields: Vec::new(),
        field_total: 0,
        row_counts: Vec::new(),
        summaries: Vec::new(),
    };
    compare_structures(expected_view, actual_view, &mut collector);
    report.container = collector.container;
    report.container_diff_total = collector.container_total;
    report.fields = collector.fields;
    report.field_diff_total = collector.field_total;
    report.row_counts = collector.row_counts;
    report.tables = collector.summaries;
    report
}

struct DiffCollector {
    limits: ReportLimits,
    format_limits: FormatLimits,
    container: Vec<ContainerDiff>,
    container_total: usize,
    fields: Vec<FieldDiff>,
    field_total: usize,
    row_counts: Vec<RowCountDiff>,
    summaries: Vec<TableDiffSummary>,
}

impl DiffCollector {
    fn push_container(&mut self, diff: ContainerDiff) {
        self.container_total += 1;
        if self.container.len() < self.limits.max_diff_entries {
            self.container.push(diff);
        }
    }

    fn push_field(&mut self, diff: FieldDiff) {
        self.field_total += 1;
        if self.fields.len() < self.limits.max_diff_entries {
            self.fields.push(diff);
        }
    }
}

fn checked_view(bytes: &[u8]) -> Result<RegistryCheckedObjectView<'_>, FormatError> {
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
    without_digest_probe(|| preflight_object_registry(bytes, kind, FormatLimits::HARD))
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

fn compare_structures(
    expected: &RegistryCheckedObjectView<'_>,
    actual: &RegistryCheckedObjectView<'_>,
    collector: &mut DiffCollector,
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
            compare_table(
                kind,
                ordinal,
                table_ordinal,
                section_id,
                expected.bytes(),
                actual.bytes(),
                &expected_table,
                &actual_table,
                collector,
            );
        }
    }
    // 逐表首差：fields 按遍历顺序收集，逐表归位首个差异。
    for diff in collector.fields.iter() {
        let summary = collector.summaries.iter_mut().find(|summary| {
            summary.section.ordinal == diff.location.section_ordinal
                && summary.table.kind == diff.location.table_kind
        });
        if let Some(summary) = summary
            && summary.first_field_diff.is_none()
        {
            summary.first_field_diff = Some(diff.clone());
        }
    }
}

fn chunk_digests_equal(
    expected: &RegistryCheckedTableView<'_>,
    actual: &RegistryCheckedTableView<'_>,
) -> bool {
    if expected.chunk_count() != actual.chunk_count() {
        return false;
    }
    (0..expected.chunk_count()).all(|chunk| {
        expected
            .chunk_digest(chunk)
            .is_some_and(|digest| actual.chunk_digest(chunk) == Some(digest))
    })
}

#[allow(clippy::too_many_arguments)]
fn compare_table(
    kind: PortableObjectKind,
    section_ordinal: u32,
    table_ordinal: u32,
    section: SectionIdentity,
    expected_object_bytes: &[u8],
    actual_object_bytes: &[u8],
    expected_table: &RegistryCheckedTableView<'_>,
    actual_table: &RegistryCheckedTableView<'_>,
    collector: &mut DiffCollector,
) {
    let table_id = table_identity(kind, section_ordinal, expected_table, table_ordinal);
    if expected_table.chunk_count() != actual_table.chunk_count() {
        collector.push_container(ContainerDiff {
            kind: ContainerKind::TableChunkCount(section, table_id),
            expected: u64::from(expected_table.chunk_count()),
            actual: u64::from(actual_table.chunk_count()),
            digests: None,
        });
    }
    for chunk in 0..expected_table.chunk_count().min(actual_table.chunk_count()) {
        if expected_table.chunk_byte_offset(chunk) != actual_table.chunk_byte_offset(chunk) {
            collector.push_container(ContainerDiff {
                kind: ContainerKind::ChunkOffset(section, table_id, chunk),
                expected: expected_table.chunk_byte_offset(chunk).unwrap_or(0),
                actual: actual_table.chunk_byte_offset(chunk).unwrap_or(0),
                digests: None,
            });
        }
        if expected_table.chunk_exact_byte_length(chunk)
            != actual_table.chunk_exact_byte_length(chunk)
        {
            collector.push_container(ContainerDiff {
                kind: ContainerKind::ChunkLength(section, table_id, chunk),
                expected: expected_table.chunk_exact_byte_length(chunk).unwrap_or(0),
                actual: actual_table.chunk_exact_byte_length(chunk).unwrap_or(0),
                digests: None,
            });
        }
        if expected_table.chunk_digest(chunk).is_some()
            && expected_table.chunk_digest(chunk) != actual_table.chunk_digest(chunk)
        {
            collector.push_container(ContainerDiff {
                kind: ContainerKind::ChunkDigest(section, table_id, chunk),
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
            identity: table_id,
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
        collector.summaries.push(TableDiffSummary {
            section,
            table: table_id,
            row_counts: (expected_table.row_count(), actual_table.row_count()),
            chunk_digests_equal: chunk_digests_equal(expected_table, actual_table),
            first_field_diff: None,
        });
        return;
    };
    let member_table = is_policy_local_change_table(kind, section_ordinal, table_ordinal);

    for logical_row in 0..expected_table.row_count().min(actual_table.row_count()) {
        let (Some(expected_row), Some(actual_row)) = (
            expected_table.row(logical_row),
            actual_table.row(logical_row),
        ) else {
            break;
        };
        compare_rows(
            &RowPath {
                section,
                table: table_id,
                logical_row,
                nested: Vec::new(),
            },
            schema.row,
            member_table,
            expected_object_bytes,
            actual_object_bytes,
            &expected_row,
            &actual_row,
            collector,
        );
    }
    collector.summaries.push(TableDiffSummary {
        section,
        table: table_id,
        row_counts: (expected_table.row_count(), actual_table.row_count()),
        chunk_digests_equal: chunk_digests_equal(expected_table, actual_table),
        first_field_diff: None,
    });
}

#[allow(clippy::too_many_arguments)]
fn compare_rows(
    path: &RowPath,
    row_schema: &PortableRowSchema,
    member_table: bool,
    expected_object_bytes: &[u8],
    actual_object_bytes: &[u8],
    expected_row: &RegistryCheckedRowView<'_>,
    actual_row: &RegistryCheckedRowView<'_>,
    collector: &mut DiffCollector,
) {
    // LFSD 策略局部变更行：两侧成员 kind 一致时才按成员登记展开。
    let member_kind = if member_table {
        match (
            policy_member_kind(*expected_row),
            policy_member_kind(*actual_row),
        ) {
            (Some(left), Some(right)) if left == right => Some(left),
            _ => None,
        }
    } else {
        None
    };
    // 字段按 tag 合并比较，不直接 zip 字段位置。
    let mut expected_fields = expected_row.fields().peekable();
    let mut actual_fields = actual_row.fields().peekable();
    loop {
        let expected_tag = expected_fields.peek().map(|field| field.tag());
        let actual_tag = actual_fields.peek().map(|field| field.tag());
        let Some(tag) = expected_tag.or(actual_tag) else {
            break;
        };
        let field_name = field_name(row_schema, tag).unwrap_or("");
        match (expected_tag, actual_tag) {
            (Some(left), Some(right)) if left == right => {
                let expected_field = expected_fields.next().expect("peek 已确认");
                let actual_field = actual_fields.next().expect("peek 已确认");
                let location =
                    path.location(Some(expected_field), tag, field_name, expected_object_bytes);
                compare_values(
                    &location,
                    member_kind,
                    row_schema,
                    expected_object_bytes,
                    actual_object_bytes,
                    &expected_field,
                    &actual_field,
                    collector,
                );
            }
            (Some(left), Some(_)) if left < actual_tag.unwrap_or(u16::MAX) => {
                let expected_field = expected_fields.next().expect("peek 已确认");
                let location =
                    path.location(Some(expected_field), tag, field_name, expected_object_bytes);
                collector.push_field(FieldDiff {
                    location,
                    expected: ValueSnapshot::Present(preview_value(
                        &expected_field,
                        &collector.limits,
                    )),
                    actual: ValueSnapshot::Missing,
                });
            }
            (_, Some(_)) => {
                let actual_field = actual_fields.next().expect("peek 已确认");
                let location =
                    path.location(Some(actual_field), tag, field_name, actual_object_bytes);
                collector.push_field(FieldDiff {
                    location,
                    expected: ValueSnapshot::Missing,
                    actual: ValueSnapshot::Present(preview_value(&actual_field, &collector.limits)),
                });
            }
            (Some(_), None) => {
                let expected_field = expected_fields.next().expect("peek 已确认");
                let location =
                    path.location(Some(expected_field), tag, field_name, expected_object_bytes);
                collector.push_field(FieldDiff {
                    location,
                    expected: ValueSnapshot::Present(preview_value(
                        &expected_field,
                        &collector.limits,
                    )),
                    actual: ValueSnapshot::Missing,
                });
            }
            (None, None) => break,
        }
    }
}

fn preview_value(field: &RegistryCheckedFieldView<'_>, limits: &ReportLimits) -> ValueDisplay {
    let mut budget = 1_usize;
    if let Ok(display) = display_field_value(*field, &FieldLocation::root(), limits, &mut budget) {
        return display;
    }
    bytes_preview(field.value_bytes(), limits)
}

#[allow(clippy::too_many_arguments)]
fn compare_values(
    location: &FieldLocation,
    member_kind: Option<u8>,
    row_schema: &PortableRowSchema,
    expected_object_bytes: &[u8],
    actual_object_bytes: &[u8],
    expected_field: &RegistryCheckedFieldView<'_>,
    actual_field: &RegistryCheckedFieldView<'_>,
    collector: &mut DiffCollector,
) {
    // LFSD 已登记成员 Bytes：两侧按同一成员 kind 展开内嵌行，递归到叶。
    if let Some(kind) = member_kind
        && matches!(expected_field.field_type(), PortableFieldType::Bytes)
        && matches!(expected_field.tag(), 5 | 6)
    {
        let expected_member =
            checked_policy_member_row(kind, expected_field.value_bytes(), collector.format_limits);
        let actual_member =
            checked_policy_member_row(kind, actual_field.value_bytes(), collector.format_limits);
        if let (Ok(expected_row), Ok(actual_row)) = (expected_member, actual_member) {
            let schema = laneflow_static_contract::policy_local_value_schema(kind)
                .unwrap_or(&EMPTY_ROW_SCHEMA);
            let nested = RowPath {
                section: SectionIdentity {
                    ordinal: location.section_ordinal,
                    name: location.section_name,
                },
                table: TableIdentity {
                    kind: location.table_kind,
                    name: location.table_name,
                },
                logical_row: location.logical_row,
                nested: {
                    let mut nested = location.nested.clone();
                    nested.push(NestedStep::PolicyMember {
                        kind,
                        field_tag: expected_field.tag(),
                        field_name: location.field_name,
                    });
                    nested
                },
            };
            compare_rows(
                &nested,
                schema,
                false,
                expected_object_bytes,
                actual_object_bytes,
                &expected_row,
                &actual_row,
                collector,
            );
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
        let base = RowPath {
            section: SectionIdentity {
                ordinal: location.section_ordinal,
                name: location.section_name,
            },
            table: TableIdentity {
                kind: location.table_kind,
                name: location.table_name,
            },
            logical_row: location.logical_row,
            nested: location.nested.clone(),
        };
        let nested_schema = row_schema
            .fields
            .iter()
            .find(|field| field.tag == expected_field.tag())
            .and_then(|field| field.nested_row)
            .unwrap_or(&EMPTY_ROW_SCHEMA);
        for index in 0..expected_vector.len() {
            if let (Some(expected_row), Some(actual_row)) =
                (expected_vector.row(index), actual_vector.row(index))
                && !rows_fieldwise_equal(&expected_row, &actual_row)
            {
                let nested = base.with_step(NestedStep::RecordVectorItem {
                    index,
                    field_tag: location.field_tag,
                    field_name: location.field_name,
                });
                compare_rows(
                    &nested,
                    nested_schema,
                    false,
                    expected_object_bytes,
                    actual_object_bytes,
                    &expected_row,
                    &actual_row,
                    collector,
                );
            }
        }
        return;
    }

    if !values_equal(*expected_field, *actual_field) {
        collector.push_field(FieldDiff {
            location: location.clone(),
            expected: ValueSnapshot::Present(preview_value(expected_field, &collector.limits)),
            actual: ValueSnapshot::Present(preview_value(actual_field, &collector.limits)),
        });
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
                && (0..left.len()).all(|index| match (left.row(index), right.row(index)) {
                    (Some(expected_row), Some(actual_row)) => {
                        rows_fieldwise_equal(&expected_row, &actual_row)
                    }
                    _ => false,
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
    hex_string(&[byte], 2)
}
