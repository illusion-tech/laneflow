//! diff 集成测试：三类差异、方向、嵌套路径、双侧观察与预算截断。

mod common;

use common::{lfca_with_identity_field_values, lfsm_with_source_locations, minimal_object};
use laneflow_artifact_inspect::diff::diff_bytes;
use laneflow_artifact_inspect::diff::{ByteDivergence, FieldSide, Side};
use laneflow_artifact_inspect::render::render_diff;
use laneflow_artifact_inspect::session::{
    DisplayPolicy, InspectFilters, ReportLimits, SectionSelector, SessionConfig,
};
use laneflow_static_contract::PortableObjectKind;

fn diff(expected: &[u8], actual: &[u8]) -> laneflow_artifact_inspect::diff::DiffReport {
    diff_bytes(
        expected,
        actual,
        &DisplayPolicy::default(),
        &SessionConfig::hardened(),
    )
}

fn render(report: &laneflow_artifact_inspect::diff::DiffReport) -> String {
    render_diff(report, &DisplayPolicy::default())
}

#[test]
fn identical_objects_are_equal() {
    let bytes = minimal_object(PortableObjectKind::SemanticDiff);
    let report = diff(&bytes, &bytes);
    assert!(report.equal());
    assert!(report.bytes_equal);
    assert!(report.byte_divergence.is_none());
    let text = render(&report);
    assert!(text.contains("bytes: 相同"));
    assert!(text.contains("conclusion: 对象相同"));
}

#[test]
fn source_location_startline_diff_reports_path_with_direction() {
    // 正式编码路径生成两份均通过预检的对象；仅来源行号不同，区间关系保持合法。
    let expected = lfsm_with_source_locations(&[3, 12, 13, 14, 15, 16, 17, 18, 19, 20]);
    let actual = lfsm_with_source_locations(&[3, 12, 13, 14, 15, 16, 17, 9, 19, 20]);
    let report = diff(&expected, &actual);
    assert!(!report.equal());
    let text = render(&report);
    assert!(
        text.contains("SourceModules.SourceLocation[7].startLine：18 → 9"),
        "应定位 startLine 且方向 expected→actual：\n{text}"
    );
    // 目录摘要变化不得遮住字段首差：逐表汇总应给出表内首差。
    assert!(
        text.contains("SourceModules[1].SourceLocation") && text.contains("表内首差"),
        "逐表汇总应含 SourceLocation 的首差：\n{text}"
    );
}

#[test]
fn row_count_change_reports_counts_and_divergence() {
    let expected = lfsm_with_source_locations(&[10, 11, 12]);
    let actual = lfsm_with_source_locations(&[10, 11]);
    let report = diff(&expected, &actual);
    assert!(!report.equal());
    assert!(
        report
            .row_counts
            .iter()
            .any(|row| row.table.name == "SourceLocation" && row.expected == 3 && row.actual == 2)
    );
    // 两份合法对象长度不同：objectByteLength 在前导内先行分歧。
    assert!(report.byte_divergence.is_some());
}

#[test]
fn nested_record_vector_diff_reports_full_path_and_side_offsets() {
    let expected = lfca_with_identity_field_values(&[b"alpha", b"beta"]);
    let actual = lfca_with_identity_field_values(&[b"alpha", b"gamma"]);
    let report = diff(&expected, &actual);
    assert!(!report.equal());
    let text = render(&report);
    assert!(
        text.contains("CanonicalIdentity[0].identityFields[1].value"),
        "嵌套 RecordVector 应沿完整路径定位：\n{text}"
    );
    assert!(
        text.contains("62657461") || text.contains("67616d6d61"),
        "应有 Bytes 预览"
    );
}

#[test]
fn side_offsets_track_each_side_independently() {
    // 评审例：变长前缀移位后，actual 偏移必须与 expected 各自独立。
    let expected = lfca_with_identity_field_values(&[b"a", b"b"]);
    let actual = lfca_with_identity_field_values(&[b"longer", b"c"]);
    let report = diff(&expected, &actual);
    // 移位的是第二个向量项：前缀项（index 0）在两侧同偏移，后继项分道。
    let is_second_item = |entry: &laneflow_artifact_inspect::diff::FieldDiff| {
        entry.path.field_name == "value"
            && entry.path.nested.iter().any(|step| {
                matches!(
                    step,
                    laneflow_artifact_inspect::walk::NestedStep::RecordVectorItem { index: 1, .. }
                )
            })
    };
    let diff = report
        .fields
        .iter()
        .find(|entry| is_second_item(entry))
        .or_else(|| {
            report
                .tables
                .iter()
                .filter_map(|t| t.first_field_diff.as_ref())
                .find(|entry| is_second_item(entry))
        })
        .expect("应有第二向量项的 value 差异");
    let (
        FieldSide::Present {
            offset: expected_offset,
            ..
        },
        FieldSide::Present {
            offset: actual_offset,
            ..
        },
    ) = (&diff.expected, &diff.actual)
    else {
        panic!("两侧均应存在 value 字段：{diff:?}");
    };
    // 同一逻辑位置在两份对象内的物理偏移随前缀长度分道。
    assert!(
        actual_offset > expected_offset,
        "actual 偏移应反映其自身前缀：expected@{expected_offset} actual@{actual_offset}"
    );
    let text = render(&report);
    assert!(
        !text.contains(&format!(
            "expected@{actual_offset}（actual@{actual_offset}）"
        )),
        "渲染不得把同一偏移标给两侧：\n{text}"
    );
}

#[test]
fn kind_mismatch_is_incomplete_not_a_field_diff() {
    let expected = minimal_object(PortableObjectKind::CanonicalArtifact);
    let actual = minimal_object(PortableObjectKind::SourceMap);
    let report = diff(&expected, &actual);
    assert!(report.incomplete());
    assert!(report.fields.is_empty());
    let text = render(&report);
    assert!(text.contains("两侧对象种类不一致"));
}

#[test]
fn same_invalid_bytes_report_no_byte_diff_but_check_failure() {
    let mut bytes = minimal_object(PortableObjectKind::SourceMap);
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    let report = diff(&bytes, &bytes);
    assert!(report.bytes_equal);
    assert!(report.incomplete());
    let text = render(&report);
    assert!(text.contains("bytes: 相同"));
    assert!(text.contains("无法完成比较"));
    assert!(text.contains("结构局部坐标"));
}

#[test]
fn truncated_side_still_reports_byte_divergence() {
    let expected = minimal_object(PortableObjectKind::SourceMap);
    let actual = expected[..expected.len() - 4].to_vec();
    let report = diff(&expected, &actual);
    assert!(report.incomplete());
    assert_eq!(
        report.byte_divergence,
        Some(ByteDivergence::Eof {
            offset: u64::try_from(actual.len()).unwrap(),
            longer_side: Side::Expected,
        })
    );
}

#[test]
fn filtered_range_without_diffs_states_full_object_verdict() {
    let expected = lfsm_with_source_locations(&[3, 12]);
    let actual = lfsm_with_source_locations(&[3, 13]);
    let policy = DisplayPolicy {
        filters: InspectFilters {
            section: Some(SectionSelector::Name("SourceMapBindings".to_string())),
            ..Default::default()
        },
        limits: ReportLimits::default(),
    };
    let report = diff_bytes(&expected, &actual, &policy, &SessionConfig::hardened());
    let text = render_diff(&report, &policy);
    assert!(
        text.contains("选定范围未发现差异；全对象比较结论为「对象不同」"),
        "筛选只影响展示：\n{text}"
    );
}

#[test]
fn diff_entry_budget_truncates_with_explicit_marker() {
    let expected_lines: Vec<u32> = (0..50).collect();
    let mut actual_lines = expected_lines.clone();
    for line in &mut actual_lines {
        *line += 1;
    }
    let expected = lfsm_with_source_locations(&expected_lines);
    let actual = lfsm_with_source_locations(&actual_lines);
    let report = diff(&expected, &actual);
    assert!(report.field_diff_total >= 50);
    let policy = DisplayPolicy {
        filters: InspectFilters::default(),
        limits: ReportLimits {
            max_diff_entries: 3,
            ..ReportLimits::default()
        },
    };
    let text = render_diff(&report, &policy);
    assert!(
        text.contains(&format!(
            "字段差异共 {} 条，展示前 3 条",
            report.field_diff_total
        )),
        "应明确标记条数截断：\n{text}"
    );
}

#[test]
fn output_byte_budget_truncates_with_explicit_marker() {
    let expected_lines: Vec<u32> = (0..50).collect();
    let mut actual_lines = expected_lines.clone();
    for line in &mut actual_lines {
        *line += 1;
    }
    let expected = lfsm_with_source_locations(&expected_lines);
    let actual = lfsm_with_source_locations(&actual_lines);
    let report = diff(&expected, &actual);
    let policy = DisplayPolicy {
        filters: InspectFilters::default(),
        limits: ReportLimits {
            max_output_bytes: 400,
            ..ReportLimits::default()
        },
    };
    let text = render_diff(&report, &policy);
    assert!(
        text.contains("输出字节限额截断，比较与检查结论不受影响"),
        "应标记输出截断：\n{text}"
    );
    assert!(text.len() < 600, "截断后正文应保持有界：{}", text.len());
}

#[test]
fn lfsd_embedded_member_diff_descends_to_member_fields() {
    let expected = common::lfsd_with_evidence_change("k", "locator-a", "locator-a");
    let actual = common::lfsd_with_evidence_change("k", "locator-a", "locator-b");
    let report = diff(&expected, &actual);
    assert!(!report.equal());
    let text = render(&report);
    assert!(
        text.contains("afterValue{member=0}.locator")
            && text.contains("\"locator-a\"")
            && text.contains("\"locator-b\""),
        "已登记成员 Bytes 差异应下钻到内嵌字段：\n{text}"
    );
}

#[test]
fn lfcp_tables_report_not_applicable_digests() {
    // 评审例：LFCP 无 chunk 目录，逐表摘要不得把「无摘要」当「不同」。
    // 两份合法对象仅在 provenance 文本上不同。
    let expected = common::lfcp_with_build_note("build-a");
    let actual = common::lfcp_with_build_note("build-b");
    let report = diff(&expected, &actual);
    assert!(!report.equal());
    assert!(report.tables.iter().all(|table| matches!(
        table.chunk_digests,
        laneflow_artifact_inspect::diff::DigestComparison::NotApplicable
    )));
    let text = render(&report);
    assert!(
        text.contains("chunk 摘要不适用（singleton）"),
        "LFCP 逐表摘要应标不适用：
{text}"
    );
    assert!(
        !text.contains("chunk 摘要不同"),
        "LFCP 不得误报摘要不同：
{text}"
    );
    assert!(report.field_diff_total > 0, "provenance 差异应落在字段层");
}
