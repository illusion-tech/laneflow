//! 稳定报告的有界渲染。
//!
//! 渲染逐行写入并受 `max_output_bytes` 约束：先限制再展开，截断时输出明确
//! 标记行。同一工具版本、相同输入与相同选项的渲染正文逐字节一致：固定遍历
//! 顺序、固定换行与值格式，不注入工作目录、时间或平台信息。
//!
//! 筛选契约：wire 事实（检查结果、字节差异、结论、全对象总数）不经过
//! [`DisplayPolicy`]——代码上没有应用筛选的路径；全部可展示类别（容器、
//! 字段、行数、表汇总）统一经过策略谓词，没有类别能绕过。

use laneflow_format::FormatError;
use laneflow_static_contract::PortableObjectKind;

use crate::diff::{
    ByteDivergence, ContainerDiff, ContainerKind, DiffReport, DigestComparison, FieldDiff,
    FieldSide, Side,
};
use crate::inspect::{InspectOptions, InspectReport};
use crate::session::DisplayPolicy;
use crate::value::ValueDisplay;

/// 对象种类的稳定中文名。
#[must_use]
pub fn object_kind_name(kind: PortableObjectKind) -> &'static str {
    match kind {
        PortableObjectKind::CanonicalArtifact => "LFCA（可移植规范制品）",
        PortableObjectKind::SourceMap => "LFSM（源映射封套）",
        PortableObjectKind::SemanticDiff => "LFSD（语义差异封套）",
        PortableObjectKind::CanonicalPublicationDescriptor => "LFCP（规范发布描述符）",
    }
}

/// 渲染 inspect 报告（应用展示策略）。
#[must_use]
pub fn render_inspect(
    report: &InspectReport,
    options: &InspectOptions,
    policy: &DisplayPolicy,
) -> String {
    let mut out = String::new();
    render_inspect_into(&mut out, report, options, policy);
    out
}

/// 渲染 inspect 报告到既有缓冲（受 `max_output_bytes` 约束）。
pub fn render_inspect_into(
    out: &mut String,
    report: &InspectReport,
    options: &InspectOptions,
    policy: &DisplayPolicy,
) {
    let mut writer = BoundedWriter::new(out, policy.limits.max_output_bytes);
    render_inspect_body(&mut writer, report, options, policy);
    writer.finish();
}

/// 渲染 diff 报告（应用展示策略）。
#[must_use]
pub fn render_diff(report: &DiffReport, policy: &DisplayPolicy) -> String {
    let mut out = String::new();
    render_diff_into(&mut out, report, policy);
    out
}

/// 渲染 diff 报告到既有缓冲（受 `max_output_bytes` 约束）。
pub fn render_diff_into(out: &mut String, report: &DiffReport, policy: &DisplayPolicy) {
    let mut writer = BoundedWriter::new(out, policy.limits.max_output_bytes);
    render_diff_body(&mut writer, report, policy);
    writer.finish();
}

struct BoundedWriter<'a> {
    out: &'a mut String,
    remaining: usize,
    truncated: bool,
}

impl<'a> BoundedWriter<'a> {
    fn new(out: &'a mut String, budget: usize) -> Self {
        let remaining = budget.saturating_sub(out.len());
        Self {
            out,
            remaining,
            truncated: false,
        }
    }

    fn line(&mut self, content: &str) {
        if self.remaining == 0 {
            self.truncated = true;
            return;
        }
        let payload = format!("{content}\n");
        if payload.len() > self.remaining {
            self.truncated = true;
            self.remaining = 0;
            return;
        }
        self.out.push_str(&payload);
        self.remaining -= payload.len();
    }

    fn finish(self) {
        if self.truncated {
            self.out
                .push_str("…（输出字节限额截断，比较与检查结论不受影响）\n");
        }
    }
}

fn render_inspect_body(
    writer: &mut BoundedWriter<'_>,
    report: &InspectReport,
    options: &InspectOptions,
    policy: &DisplayPolicy,
) {
    let facts = &report.facts;
    // 摘要与前导是 wire 事实：不经过筛选谓词。
    writer.line(&format!("object: {}", object_kind_name(facts.kind)));
    writer.line(&format!("formatVersion: {}", facts.format_version));
    writer.line(&format!("bytes: {}", facts.object_byte_length));
    writer.line(&format!(
        "sections: {} / tables: {} / rows: {}",
        facts.section_count, facts.table_count, facts.row_count
    ));
    match facts.chunk_count {
        Some(count) => writer.line(&format!("chunks: {count}")),
        None => writer.line("chunks: 不适用（LFCP singleton 节无 chunk 目录）"),
    }
    writer.line("check: 单对象结构与直接值域检查 PASS");
    writer.line("scope: 跨对象绑定、运行时静态路网构建与发布真实性不在本工具检查范围");

    if options.expand.preamble {
        writer.line("preamble:");
        writer.line(&format!(
            "  magic: {}",
            crate::value::hex_string(&facts.kind.magic(), 8)
        ));
        writer.line(&format!("  formatVersion: {}", facts.format_version));
        writer.line("  headerByteLength: 32");
        writer.line("  flags: 0");
        writer.line(&format!("  sectionCount: {}", facts.section_count));
        writer.line("  sectionDirectoryOffset: 32");
        writer.line(&format!("  objectByteLength: {}", facts.object_byte_length));
    }

    if options.expand.sections {
        writer.line("sections:");
        for section in &report.sections {
            writer.line(&format!(
                "  [{}] {} v{} offset={} length={} tables={} rows={}",
                section.identity.ordinal,
                section.identity.name,
                section.format_version,
                section.byte_offset,
                section.byte_length,
                section.table_count,
                section.row_count
            ));
        }
    }

    if options.expand.chunks {
        writer.line("chunks:");
        for section in &report.sections {
            for table in &section.tables {
                if table.no_chunk_directory {
                    writer.line(&format!(
                        "  {}.{} singleton（chunk directory 不适用）",
                        section.identity.name, table.identity.name
                    ));
                    continue;
                }
                for chunk in &table.chunks {
                    writer.line(&format!(
                        "  {}.{} chunk[{}] index={} firstRow={} rows={} offset={} length={} digest={}",
                        section.identity.name,
                        table.identity.name,
                        chunk.index,
                        chunk.index,
                        chunk.first_logical_row,
                        chunk.row_count,
                        chunk.byte_offset,
                        chunk.byte_length,
                        chunk
                            .digest
                            .as_ref()
                            .map_or_else(|| "—".to_string(), crate::value::digest_hex)
                    ));
                }
            }
        }
    }

    if options.expand.fields {
        writer.line("fields:");
        for entry in &report.fields {
            writer.line(&format!(
                "  {} = {}",
                entry.path.render(),
                render_value(&entry.value)
            ));
        }
        for (table, remaining) in &report.truncated_tables {
            writer.line(&format!(
                "  …（{} 因行数限额截断，剩余 {remaining} 行未展开）",
                table.name
            ));
        }
    }
    let _ = policy;
}

fn render_diff_body(writer: &mut BoundedWriter<'_>, report: &DiffReport, policy: &DisplayPolicy) {
    // 检查状态、字节差异与结论是事实：不经过筛选谓词。
    writer.line("check:");
    writer.line(&format!(
        "  expected: {}",
        render_side(&report.expected.check)
    ));
    writer.line(&format!(
        "  actual:   {}",
        render_side(&report.actual.check)
    ));

    match report.byte_divergence {
        None => writer.line("bytes: 相同"),
        Some(ByteDivergence::Offset {
            offset,
            expected,
            actual,
        }) => writer.line(&format!(
            "bytes: 不同（首个分歧 offset={offset}：expected {} / actual {}）",
            crate::diff::byte_hex(expected),
            crate::diff::byte_hex(actual)
        )),
        Some(ByteDivergence::Eof {
            offset,
            longer_side,
        }) => {
            let side = match longer_side {
                Side::Expected => "expected",
                Side::Actual => "actual",
            };
            writer.line(&format!(
                "bytes: 不同（前缀相同，{side} 侧在 offset={offset} 之后仍有字节）"
            ));
        }
    }

    if report.both_checked_same_kind {
        writer.line(if report.bytes_equal {
            "conclusion: 对象相同（精确字节相等且两侧检查通过）"
        } else {
            "conclusion: 对象不同（精确字节相等是相等判据）"
        });
    } else if matches!(
        (&report.expected.check, &report.actual.check),
        (Ok(left), Ok(right)) if left != right
    ) {
        writer.line("conclusion: 无法完成比较（两侧对象种类不一致）");
    } else {
        writer.line("conclusion: 无法完成比较（存在预检失败的一侧；字节差异仍如上报告）");
        writer.line(&format!(
            "  expected: {}",
            render_format_error(report.expected.check.as_ref().err())
        ));
        writer.line(&format!(
            "  actual:   {}",
            render_format_error(report.actual.check.as_ref().err())
        ));
        writer.line("  （FormatError 偏移为正式读取器的结构局部坐标或未知，不是文件绝对位置）");
    }

    // 以下全部是可展示类别：统一经过策略谓词。
    let container_selected: Vec<&ContainerDiff> = report
        .container
        .iter()
        .filter(|diff| container_selected(diff, policy))
        .collect();
    let fields_selected: Vec<&FieldDiff> = report
        .fields
        .iter()
        .take(policy.limits.max_diff_entries)
        .collect();

    if report.container_diff_total > 0 || !report.row_counts.is_empty() {
        writer.line("container:");
        for diff in container_selected.iter() {
            writer.line(&render_container(diff));
        }
        // 截断协议：容器差异被条数预算省略时明确标记，与字段通道对称。
        if report.container_diff_total > container_selected.len() {
            writer.line(&format!(
                "  …（容器差异共 {} 条，展示前 {} 条）",
                report.container_diff_total,
                container_selected.len()
            ));
        }
        for row in &report.row_counts {
            if policy
                .filters
                .selects_section(row.section.ordinal, row.section.name)
                && policy.filters.selects_table(row.table.kind, row.table.name)
            {
                writer.line(&format!(
                    "  {}[{}].{}：行数 {} → {}",
                    row.section.name, row.section.ordinal, row.table.name, row.expected, row.actual
                ));
            }
        }
    }

    if report.field_diff_total > 0 {
        writer.line(&format!(
            "fields: 共 {} 条（按节、表、逻辑行、tag 顺序）",
            report.field_diff_total
        ));
        for diff in fields_selected.iter() {
            writer.line(&render_field_diff(diff));
        }
        if report.field_diff_total > fields_selected.len() {
            writer.line(&format!(
                "  …（字段差异共 {} 条，展示前 {} 条）",
                report.field_diff_total,
                fields_selected.len()
            ));
        }
    } else if report.both_checked_same_kind && !report.bytes_equal {
        writer.line("fields: 无字段差异（差异在容器或字节层）");
    }

    let tables_selected: Vec<&crate::diff::TableDiffSummary> = report
        .tables
        .iter()
        .filter(|summary| {
            policy
                .filters
                .selects_section(summary.section.ordinal, summary.section.name)
                && policy
                    .filters
                    .selects_table(summary.table.kind, summary.table.name)
        })
        .collect();
    if !report.tables.is_empty() {
        writer.line("tables:");
        for summary in tables_selected {
            let digest_note = match &summary.chunk_digests {
                DigestComparison::Equal => "chunk 摘要一致",
                DigestComparison::NotApplicable => "chunk 摘要不适用（singleton）",
                DigestComparison::Differ { .. } => "chunk 摘要不同",
            };
            let rows_note = if summary.row_counts.0 == summary.row_counts.1 {
                format!("rows {}={}", summary.row_counts.0, summary.row_counts.1)
            } else {
                format!("rows {}→{}", summary.row_counts.0, summary.row_counts.1)
            };
            let first = summary
                .first_field_diff
                .as_ref()
                .map_or_else(String::new, |diff| {
                    format!("；表内首差 {}", render_field_diff_inline(diff))
                });
            writer.line(&format!(
                "  {}[{}].{}：{rows_note}，{digest_note}{first}",
                summary.section.name, summary.section.ordinal, summary.table.name
            ));
        }
    }

    if policy.filters.is_active() {
        let any_shown = !container_selected.is_empty()
            || !fields_selected.is_empty()
            || report.row_counts.iter().any(|row| {
                policy
                    .filters
                    .selects_section(row.section.ordinal, row.section.name)
                    && policy.filters.selects_table(row.table.kind, row.table.name)
            })
            || report.tables.iter().any(|summary| {
                summary.first_field_diff.is_some()
                    && policy
                        .filters
                        .selects_section(summary.section.ordinal, summary.section.name)
                    && policy
                        .filters
                        .selects_table(summary.table.kind, summary.table.name)
            });
        if !any_shown && report.both_checked_same_kind && !report.bytes_equal {
            writer
                .line("range: 选定范围未发现差异；全对象比较结论为「对象不同」（筛选只影响展示）");
        }
    }
}

fn render_side(check: &Result<PortableObjectKind, FormatError>) -> String {
    match check {
        Ok(kind) => format!(
            "PASS（{} v{}）",
            object_kind_name(*kind),
            kind.format_version()
        ),
        Err(error) => format!("FAIL（{}）", crate::diff::format_error_display(error)),
    }
}

fn render_format_error(error: Option<&FormatError>) -> String {
    error.map_or_else(|| "—".to_string(), crate::diff::format_error_display)
}

fn container_selected(diff: &ContainerDiff, policy: &DisplayPolicy) -> bool {
    match (diff.section(), diff.table()) {
        (None, None) => true,
        (Some(section), None) => policy
            .filters
            .selects_section(section.ordinal, section.name),
        (Some(section), Some(table)) => {
            policy
                .filters
                .selects_section(section.ordinal, section.name)
                && policy.filters.selects_table(table.kind, table.name)
        }
        (None, Some(_)) => false,
    }
}

fn render_container(diff: &ContainerDiff) -> String {
    let label = match &diff.kind {
        ContainerKind::ObjectLength => "对象长度".to_string(),
        ContainerKind::SectionOffset(section) => {
            format!("节[{}].{} 偏移", section.ordinal, section.name)
        }
        ContainerKind::SectionLength(section) => {
            format!("节[{}].{} 长度", section.ordinal, section.name)
        }
        ContainerKind::TableChunkCount(section, table) => {
            format!(
                "{}[{}].{} chunk 数",
                section.name, section.ordinal, table.name
            )
        }
        ContainerKind::ChunkOffset(section, table, chunk) => format!(
            "{}[{}].{} chunk[{chunk}] 偏移",
            section.name, section.ordinal, table.name
        ),
        ContainerKind::ChunkLength(section, table, chunk) => format!(
            "{}[{}].{} chunk[{chunk}] 长度",
            section.name, section.ordinal, table.name
        ),
        ContainerKind::ChunkDigest(section, table, chunk) => {
            if let Some((expected, actual)) = diff.digests {
                return format!(
                    "  {}[{}].{} chunk[{chunk}] 摘要 {} → {}",
                    section.name,
                    section.ordinal,
                    table.name,
                    crate::value::digest_hex(&expected),
                    crate::value::digest_hex(&actual)
                );
            }
            format!(
                "{}[{}].{} chunk[{chunk}] 摘要",
                section.name, section.ordinal, table.name
            )
        }
    };
    format!("  {label} {} → {}", diff.expected, diff.actual)
}

fn render_field_diff(diff: &FieldDiff) -> String {
    format!("  {}", render_field_diff_inline(diff))
}

fn render_field_diff_inline(diff: &FieldDiff) -> String {
    let expected = render_side_value(&diff.expected);
    let actual = render_side_value(&diff.actual);
    let mut note = String::new();
    if let FieldSide::Present { offset, .. } = &diff.expected {
        note.push_str(&format!("（expected@{offset}）"));
    }
    if let FieldSide::Present { offset, .. } = &diff.actual {
        note.push_str(&format!("（actual@{offset}）"));
    }
    format!("{}：{expected} → {actual}{note}", diff.path.render())
}

fn render_side_value(side: &FieldSide) -> String {
    match side {
        FieldSide::Missing => "缺失".to_string(),
        FieldSide::Present { value, .. } => render_value(value),
    }
}

/// 渲染一个值展示快照（确定性）。
#[must_use]
pub fn render_value(value: &ValueDisplay) -> String {
    match value {
        ValueDisplay::U8(v) => format!("{v}"),
        ValueDisplay::U16(v) => format!("{v}"),
        ValueDisplay::U32(v) => format!("{v}"),
        ValueDisplay::U64(v) => format!("{v}"),
        ValueDisplay::I32(v) => format!("{v}"),
        ValueDisplay::F32 { decimal, bits } => format!("{decimal}（位模式 0x{bits:08x}）"),
        ValueDisplay::F64 { decimal, bits } => format!("{decimal}（位模式 0x{bits:016x}）"),
        ValueDisplay::StableId128(hex) => format!("stable-id:{hex}"),
        ValueDisplay::Sha256(hex) => format!("sha256:{hex}"),
        ValueDisplay::Utf8 {
            text,
            truncated,
            total,
        } => {
            if *truncated {
                format!("\"{text}\"…（UTF-8 共 {total} 字节，截断）")
            } else {
                format!("\"{text}\"")
            }
        }
        ValueDisplay::Bytes { hex, total, digest } => {
            if digest.is_empty() {
                format!("bytes[{total}]={hex}")
            } else {
                format!("bytes[{total}]={hex}…（sha256:{digest}）")
            }
        }
        ValueDisplay::OrdinalVector { items, total } => {
            let shown: Vec<String> = items.iter().map(u32::to_string).collect();
            if (*total as usize) > items.len() {
                format!(
                    "vector[{total}]（前 {} 项：{}，截断）",
                    items.len(),
                    shown.join(", ")
                )
            } else {
                format!("vector[{total}]：{}", shown.join(", "))
            }
        }
        ValueDisplay::RecordVector { rows, total } => {
            let shown: Vec<String> = rows
                .iter()
                .map(|fields| {
                    let inner: Vec<String> = fields
                        .iter()
                        .map(|(tag, value)| format!("{}={}", tag, render_value(value)))
                        .collect();
                    format!("{{{}}}", inner.join(", "))
                })
                .collect();
            if (*total as usize) > rows.len() {
                format!(
                    "records[{total}]（前 {} 项：{}，截断）",
                    rows.len(),
                    shown.join("; ")
                )
            } else {
                format!("records[{total}]：{}", shown.join("; "))
            }
        }
        ValueDisplay::EmbeddedRow { kind, fields } => match fields {
            None => format!("member{{kind={kind}}}（行数限额截断，内容未展开）"),
            Some(fields) => {
                let inner: Vec<String> = fields
                    .iter()
                    .map(|(tag, value)| format!("{}={}", tag, render_value(value)))
                    .collect();
                format!("member{{kind={kind}}}{{{}}}", inner.join(", "))
            }
        },
    }
}
