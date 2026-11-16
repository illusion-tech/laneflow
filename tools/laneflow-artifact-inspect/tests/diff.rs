//! diff 集成测试：三类差异、方向、嵌套路径、缺失字段与预算截断。

mod common;

use common::{lfca_with_identity_field_values, lfsm_with_source_locations, minimal_object};
use laneflow_artifact_inspect::budget::ReportLimits;
use laneflow_artifact_inspect::diff::{DiffOptions, diff_bytes};
use laneflow_artifact_inspect::render::render_diff;
use laneflow_static_contract::PortableObjectKind;

#[test]
fn identical_objects_are_equal() {
    let bytes = minimal_object(PortableObjectKind::SemanticDiff);
    let report = diff_bytes(&bytes, &bytes);
    assert!(report.equal());
    assert!(report.bytes_equal);
    assert!(report.byte_divergence.is_none());
    let text = render_diff(&report, &DiffOptions::default(), &ReportLimits::default());
    assert!(text.contains("bytes: 相同"));
    assert!(text.contains("conclusion: 对象相同"));
}

#[test]
fn source_location_startline_diff_reports_path_with_direction() {
    // 正式编码路径生成两份均通过预检的对象；仅来源行号不同，区间关系保持合法。
    let expected = lfsm_with_source_locations(&[3, 12, 13, 14, 15, 16, 17, 18, 19, 20]);
    let actual = lfsm_with_source_locations(&[3, 12, 13, 14, 15, 16, 17, 9, 19, 20]);
    let report = diff_bytes(&expected, &actual);
    assert!(!report.equal());
    let text = render_diff(&report, &DiffOptions::default(), &ReportLimits::default());
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
fn row_count_change_reports_counts_and_eof_style_divergence() {
    let expected = lfsm_with_source_locations(&[10, 11, 12]);
    let actual = lfsm_with_source_locations(&[10, 11]);
    let report = diff_bytes(&expected, &actual);
    assert!(!report.equal());
    assert!(
        report
            .row_counts
            .iter()
            .any(|row| row.identity.name == "SourceLocation"
                && row.expected == 3
                && row.actual == 2)
    );
    // 两份合法对象长度不同：objectByteLength 在前导内先行分歧。
    assert!(report.byte_divergence.is_some());
}

#[test]
fn nested_record_vector_diff_reports_full_path() {
    let expected = lfca_with_identity_field_values(&[b"alpha", b"beta"]);
    let actual = lfca_with_identity_field_values(&[b"alpha", b"gamma"]);
    let report = diff_bytes(&expected, &actual);
    assert!(!report.equal());
    let text = render_diff(&report, &DiffOptions::default(), &ReportLimits::default());
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
fn kind_mismatch_is_incomplete_not_a_field_diff() {
    let expected = minimal_object(PortableObjectKind::CanonicalArtifact);
    let actual = minimal_object(PortableObjectKind::SourceMap);
    let report = diff_bytes(&expected, &actual);
    assert!(report.incomplete());
    assert!(report.fields.is_empty());
    let text = render_diff(&report, &DiffOptions::default(), &ReportLimits::default());
    assert!(text.contains("两侧对象种类不一致"));
}

#[test]
fn same_invalid_bytes_report_no_byte_diff_but_check_failure() {
    let mut bytes = minimal_object(PortableObjectKind::SourceMap);
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    let report = diff_bytes(&bytes, &bytes);
    assert!(report.bytes_equal);
    assert!(report.incomplete());
    let text = render_diff(&report, &DiffOptions::default(), &ReportLimits::default());
    assert!(text.contains("bytes: 相同"));
    assert!(text.contains("无法完成比较"));
    assert!(text.contains("结构局部坐标"));
}

#[test]
fn truncated_side_still_reports_byte_divergence() {
    let expected = minimal_object(PortableObjectKind::SourceMap);
    let actual = expected[..expected.len() - 4].to_vec();
    let report = diff_bytes(&expected, &actual);
    assert!(report.incomplete());
    assert_eq!(
        report.byte_divergence,
        Some(laneflow_artifact_inspect::diff::ByteDivergence::Eof {
            offset: u64::try_from(actual.len()).unwrap(),
            longer_side: laneflow_artifact_inspect::diff::Side::Expected,
        })
    );
}

#[test]
fn filtered_range_without_diffs_states_full_object_verdict() {
    let expected = lfsm_with_source_locations(&[3, 12]);
    let actual = lfsm_with_source_locations(&[3, 13]);
    let options = DiffOptions {
        filters: laneflow_artifact_inspect::inspect::InspectFilters {
            section: Some(laneflow_artifact_inspect::inspect::SectionSelector::Name(
                "SourceMapBindings".to_string(),
            )),
            ..Default::default()
        },
    };
    let report = diff_bytes(&expected, &actual);
    let text = render_diff(&report, &options, &ReportLimits::default());
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
    let report = diff_bytes(&expected, &actual);
    assert!(report.field_diff_total >= 50);
    let limits = ReportLimits {
        max_diff_entries: 3,
        ..ReportLimits::default()
    };
    let text = render_diff(&report, &DiffOptions::default(), &limits);
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
    let report = diff_bytes(&expected, &actual);
    let limits = ReportLimits {
        max_output_bytes: 400,
        ..ReportLimits::default()
    };
    let text = render_diff(&report, &DiffOptions::default(), &limits);
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
    let report = diff_bytes(&expected, &actual);
    assert!(!report.equal());
    let text = render_diff(&report, &DiffOptions::default(), &ReportLimits::default());
    assert!(
        text.contains("afterValue{member=0}.locator")
            && text.contains("\"locator-a\"")
            && text.contains("\"locator-b\""),
        "已登记成员 Bytes 差异应下钻到内嵌字段：\n{text}"
    );
}
