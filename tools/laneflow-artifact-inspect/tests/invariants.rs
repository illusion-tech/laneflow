//! 机制级不变量与评审复现锚：验证五个机制（双侧模型、事实/展示分层、资源
//! 一等上下文、截断协议、顺序游标）的不变量，而不是逐条补丁的回归。

mod common;

use common::{lfcp_with_controlled_fields, lfsm_with_source_locations, minimal_object};
use laneflow_artifact_inspect::diff::{FieldSide, diff_bytes};
use laneflow_artifact_inspect::inspect::{ExpandSet, InspectOptions};
use laneflow_artifact_inspect::render::{render_diff, render_inspect};
use laneflow_artifact_inspect::session::{
    DisplayPolicy, InspectFilters, ReportLimits, SectionSelector, SessionConfig,
};
use laneflow_static_contract::PortableObjectKind;

// —— 机制 C：资源策略一等公民（SessionConfig 穿透） ——

#[test]
fn tightened_budget_is_enforced_by_diff_preflight_too() {
    // 评审例：base.lfca 自比 + --max-chunks-per-section=1。inspect 与 diff
    // 必须对超限对象行为一致（都判无法完成），不允许「校验后丢弃」。
    let bytes = minimal_object(PortableObjectKind::CanonicalArtifact);
    // 收紧对象字节预算到实际大小之下：diff 的预检必须与会话一致地拒绝，
    // 而不是内部回退 HARD 后放行（最小 LFCA 约 1.5 KiB，收紧到 1 KiB）。
    let tightened = SessionConfig::tighten(Some(1024), None).expect("收紧合法");
    let report = diff_bytes(&bytes, &bytes, &DisplayPolicy::default(), &tightened);
    assert!(
        report.incomplete(),
        "diff 的预检必须使用会话收紧预算，而不是 HARD"
    );
    let hardened_report = diff_bytes(
        &bytes,
        &bytes,
        &DisplayPolicy::default(),
        &SessionConfig::hardened(),
    );
    assert!(!hardened_report.incomplete(), "HARD 会话下同一对象应通过");
}

#[test]
fn read_enforces_limit_during_read_not_after() {
    // 读入期限界：limit+1 探测字节必须在读入期拦截，而非先无界读完。
    let dir = std::env::temp_dir().join(format!(
        "laneflow-artifact-inspect-inv-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("oversized.bin");
    std::fs::write(&path, vec![0_u8; 4096]).unwrap();
    let error = laneflow_artifact_inspect::input::read_object_input(
        &path,
        laneflow_artifact_inspect::input::InputEncoding::Raw,
        laneflow_artifact_inspect::session::InputLimits::for_object_budget(1024),
    )
    .unwrap_err();
    std::fs::remove_dir_all(&dir).ok();
    assert!(
        matches!(
            error,
            laneflow_artifact_inspect::input::InputError::InputTooLarge {
                actual: 1025,
                limit: 1024
            }
        ),
        "应在读满 limit+1 字节时当场报超限：{error:?}"
    );
}

// —— 机制 A：联接键与两侧观察分离 ——

#[test]
fn actual_only_optional_field_is_reported_under_its_own_tag() {
    // 评审例：两侧都有 controlledTimestamp（tag 6），actual 另有
    // controlledBuildProvenance（tag 5）。新增字段必须按自己的 tag/名上报，
    // 不得借用下一个 expected 字段的标签。
    let expected = lfcp_with_controlled_fields("build", None, Some("2026-01-01T00:00:00Z"));
    let actual =
        lfcp_with_controlled_fields("build", Some("controlled"), Some("2026-01-01T00:00:00Z"));
    let report = diff_bytes(
        &expected,
        &actual,
        &DisplayPolicy::default(),
        &SessionConfig::hardened(),
    );
    let provenance = report
        .fields
        .iter()
        .chain(
            report
                .tables
                .iter()
                .filter_map(|table| table.first_field_diff.as_ref()),
        )
        .find(|entry| entry.path.field_name == "controlledBuildProvenance")
        .expect("actual-only 字段应按自己的登记名上报");
    assert!(matches!(provenance.expected, FieldSide::Missing));
    let FieldSide::Present { value, .. } = &provenance.actual else {
        panic!("actual 侧应存在：{provenance:?}");
    };
    let rendered = laneflow_artifact_inspect::render::render_value(value);
    assert!(rendered.contains("controlled"), "值展示错误：{rendered}");
    // 不得把新增误报到 controlledTimestamp 名下。
    let timestamp = report
        .fields
        .iter()
        .find(|entry| entry.path.field_name == "controlledTimestamp");
    assert!(timestamp.is_none(), "共有字段不得被误报：{:?}", timestamp);
}

// —— 机制 B：事实域不可筛 ——

#[test]
fn object_facts_are_invariant_under_any_filter() {
    let bytes = lfsm_with_source_locations(&[11, 12, 13]);
    let options = InspectOptions::default();
    let unfiltered = laneflow_artifact_inspect::inspect::inspect(
        &bytes,
        &options,
        &DisplayPolicy::default(),
        &SessionConfig::hardened(),
    )
    .unwrap();
    let filtered = laneflow_artifact_inspect::inspect::inspect(
        &bytes,
        &InspectOptions {
            expand: ExpandSet {
                preamble: true,
                sections: true,
                ..Default::default()
            },
        },
        &DisplayPolicy {
            filters: InspectFilters {
                section: Some(SectionSelector::Ordinal(1)),
                ..Default::default()
            },
            limits: ReportLimits::default(),
        },
        &SessionConfig::hardened(),
    )
    .unwrap();
    assert_eq!(unfiltered.facts, filtered.facts, "wire 事实不得被筛选改写");
    // 评审例：--section=0 --expand=preamble 必须报真实 sectionCount。
    let text = render_inspect(
        &filtered,
        &InspectOptions {
            expand: ExpandSet {
                preamble: true,
                ..Default::default()
            },
        },
        &DisplayPolicy {
            filters: InspectFilters {
                section: Some(SectionSelector::Ordinal(1)),
                ..Default::default()
            },
            limits: ReportLimits::default(),
        },
    );
    assert!(
        text.contains("sectionCount: 5"),
        "前导须报真实节数：\n{text}"
    );
    assert!(
        text.contains("sections: 5 / tables: 8"),
        "摘要须报真实总数：\n{text}"
    );
}

#[test]
fn collection_is_filter_aware_beyond_global_prefix() {
    // 评审例：300 条差异 + --rows=149..150。选中行的差异必须可见，
    // 「选定范围未发现差异」不得成为假话。
    let expected_lines: Vec<u32> = (0..150).collect();
    let mut actual_lines = expected_lines.clone();
    for line in &mut actual_lines {
        *line += 1;
    }
    let expected = lfsm_with_source_locations(&expected_lines);
    let actual = lfsm_with_source_locations(&actual_lines);
    let policy = DisplayPolicy {
        filters: InspectFilters {
            rows: Some(laneflow_artifact_inspect::session::RowRange {
                start: 149,
                end: 150,
            }),
            ..Default::default()
        },
        limits: ReportLimits::default(),
    };
    let report = diff_bytes(&expected, &actual, &policy, &SessionConfig::hardened());
    assert_eq!(report.field_diff_total, 300, "精确计数不受筛选影响");
    assert!(
        report
            .fields
            .iter()
            .any(|entry| entry.path.logical_row == 149 && entry.path.field_name == "startLine"),
        "选中行的差异必须进入展示通道"
    );
    let text = render_diff(&report, &policy);
    assert!(
        text.contains("SourceLocation[149].startLine：149 → 150"),
        "筛选应能揭示全局前缀之外的差异：\n{text}"
    );
    assert!(
        !text.contains("选定范围未发现差异"),
        "范围内有差异时不得声称未发现：\n{text}"
    );
}

#[test]
fn section_filter_applies_to_row_counts_and_table_summaries() {
    // 评审例：--section 下行数差异与逐表汇总会漏出未选节。
    let expected = lfsm_with_source_locations(&[10, 11, 12]);
    let actual = lfsm_with_source_locations(&[10, 11]);
    let policy = DisplayPolicy {
        filters: InspectFilters {
            section: Some(SectionSelector::Ordinal(0)),
            ..Default::default()
        },
        limits: ReportLimits::default(),
    };
    let report = diff_bytes(&expected, &actual, &policy, &SessionConfig::hardened());
    let text = render_diff(&report, &policy);
    assert!(
        !text.contains("SourceLocation") || !text.contains("行数 3 → 2"),
        "未选节的行数差异不得出现：\n{text}"
    );
    assert!(
        !text.contains("SourceModules[1].SourceLocation"),
        "未选节的表汇总不得出现：\n{text}"
    );
}

// —— 机制 D：截断协议 ——

#[test]
fn escape_expansion_marks_truncation_from_output_domain() {
    // 评审例：64 个反斜杠 + 'A'（65 输入字节）转义后 129 字节超预算，
    // 末字符被省略必须带截断标记。
    let expected = common::lfcp_with_build_note(&format!("{}{}", "\\".repeat(64), 'A'));
    let actual = common::lfcp_with_build_note(&format!("{}{}", "\\".repeat(64), 'B'));
    let report = diff_bytes(
        &expected,
        &actual,
        &DisplayPolicy::default(),
        &SessionConfig::hardened(),
    );
    let build_id = report
        .fields
        .iter()
        .find(|entry| entry.path.field_name == "publisherBuildId")
        .expect("buildId 差异应存在");
    let FieldSide::Present { value, .. } = &build_id.actual else {
        panic!("actual 侧应存在");
    };
    let rendered = laneflow_artifact_inspect::render::render_value(value);
    assert!(
        rendered.contains("截断"),
        "转义扩张超出预算必须标记截断：{rendered}"
    );
}

#[test]
fn budget_exhausted_embedded_member_is_omitted_not_empty() {
    // 评审例：行预算恰好耗尽时，内嵌成员必须是「省略」而非空内容。
    let bytes = common::lfsd_with_evidence_change("k", "locator-a", "locator-b");
    let options = InspectOptions {
        expand: ExpandSet {
            fields: true,
            ..Default::default()
        },
    };
    let policy = DisplayPolicy {
        filters: InspectFilters::default(),
        limits: ReportLimits {
            max_expanded_rows: 2,
            ..ReportLimits::default()
        },
    };
    let report = laneflow_artifact_inspect::inspect::inspect(
        &bytes,
        &options,
        &policy,
        &SessionConfig::hardened(),
    )
    .unwrap();
    let text = render_inspect(&report, &options, &policy);
    assert!(
        !text.contains("member{kind=0}{}"),
        "省略不得伪装成空成员：\n{text}"
    );
    assert!(
        text.contains("member{kind=0}（行数限额截断，内容未展开）"),
        "省略必须带明确标记：\n{text}"
    );
}

// —— 机制 E：顺序游标 ——

#[test]
fn late_row_difference_is_located_without_rescanning() {
    // 顺序游标的端到端锚：2 万行表中仅最后一行不同，必须定位到
    // 逻辑行 19999（随机访问重扫在该规模下不可行，顺序 zip 单遍可达）。
    let expected_lines: Vec<u32> = vec![7; 20_000];
    let mut actual_lines = expected_lines.clone();
    let last = actual_lines.len() - 1;
    actual_lines[last] = 8;
    let expected = lfsm_with_source_locations(&expected_lines);
    let actual = lfsm_with_source_locations(&actual_lines);
    let report = diff_bytes(
        &expected,
        &actual,
        &DisplayPolicy::default(),
        &SessionConfig::hardened(),
    );
    assert_eq!(report.field_diff_total, 2, "仅 endLine 与 startLine 各一处");
    assert!(
        report
            .fields
            .iter()
            .all(|entry| entry.path.logical_row == 19_999 || {
                // 展示通道只含末行差异；逐表首差也应在末行。
                report
                    .tables
                    .iter()
                    .filter_map(|table| table.first_field_diff.as_ref())
                    .all(|first| first.path.logical_row == 19_999)
            })
    );
}

#[test]
fn container_entry_budget_truncation_is_marked() {
    // 第三轮评审例：容器差异超过条数预算时，部分目录诊断不得伪装成完整。
    // identity 值变长会平移后续节的偏移/长度并改变 chunk 摘要，产生多条
    // 容器差异；预算收紧到 1 条后必须出现明确截断标记。
    let expected = common::lfca_with_identity_field_values(&[b"a", b"b"]);
    let actual = common::lfca_with_identity_field_values(&[b"longer", b"c"]);
    let policy = DisplayPolicy {
        filters: InspectFilters::default(),
        limits: ReportLimits {
            max_diff_entries: 1,
            ..ReportLimits::default()
        },
    };
    let report = diff_bytes(&expected, &actual, &policy, &SessionConfig::hardened());
    assert!(
        report.container_diff_total > 1,
        "本例应产生多条容器差异：{}",
        report.container_diff_total
    );
    assert_eq!(report.container.len(), 1, "展示通道受条数上限约束");
    let text = render_diff(&report, &policy);
    assert!(
        text.contains(&format!(
            "容器差异共 {} 条，展示前 1 条",
            report.container_diff_total
        )),
        "容器截断必须有明确标记：\n{text}"
    );
}
