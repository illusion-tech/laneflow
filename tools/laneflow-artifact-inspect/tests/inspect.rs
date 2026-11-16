//! inspect 集成测试：事实域摘要、展开、筛选、LFCP singleton 与错误语义。

mod common;

use common::{lfsd_with_evidence_change, lfsm_with_source_locations, minimal_object};
use laneflow_artifact_inspect::inspect::{ExpandSet, InspectError, InspectOptions};
use laneflow_artifact_inspect::render::render_inspect;
use laneflow_artifact_inspect::session::{
    DisplayPolicy, InspectFilters, ReportLimits, RowRange, SectionSelector, SessionConfig,
};
use laneflow_static_contract::PortableObjectKind;

fn fields_options() -> InspectOptions {
    InspectOptions {
        expand: ExpandSet {
            fields: true,
            ..Default::default()
        },
    }
}

fn default_policy() -> DisplayPolicy {
    DisplayPolicy::default()
}

#[test]
fn minimal_objects_of_all_kinds_render_summary() {
    for kind in PortableObjectKind::ALL {
        let bytes = minimal_object(kind);
        let options = InspectOptions::default();
        let policy = default_policy();
        let report = laneflow_artifact_inspect::inspect::inspect(
            &bytes,
            &options,
            &policy,
            &SessionConfig::hardened(),
        )
        .unwrap_or_else(|error| panic!("{kind:?}: {error:?}"));
        let text = render_inspect(&report, &options, &policy);
        assert!(
            text.contains("check: 单对象结构与直接值域检查 PASS"),
            "{kind:?} 摘要缺检查行：\n{text}"
        );
        assert!(
            text.contains("scope: 跨对象绑定、运行时静态路网构建与发布真实性不在本工具检查范围"),
            "{kind:?} 摘要缺检查范围行：\n{text}"
        );
        assert!(text.contains(&format!("formatVersion: {}", kind.format_version())));
        assert!(text.contains(&format!("sections: {}", kind.section_count())));
    }
}

#[test]
fn lfcp_summary_marks_singleton_without_chunk_directory() {
    let bytes = minimal_object(PortableObjectKind::CanonicalPublicationDescriptor);
    let options = InspectOptions::default();
    let policy = default_policy();
    let report = laneflow_artifact_inspect::inspect::inspect(
        &bytes,
        &options,
        &policy,
        &SessionConfig::hardened(),
    )
    .unwrap();
    assert_eq!(report.facts.chunk_count, None);
    let text = render_inspect(&report, &options, &policy);
    assert!(
        text.contains("chunks: 不适用（LFCP singleton 节无 chunk 目录）"),
        "LFCP 应标 singleton：\n{text}"
    );
}

#[test]
fn lfcp_chunk_expansion_marks_directory_not_applicable() {
    let bytes = minimal_object(PortableObjectKind::CanonicalPublicationDescriptor);
    let options = InspectOptions {
        expand: ExpandSet {
            chunks: true,
            ..Default::default()
        },
    };
    let policy = default_policy();
    let report = laneflow_artifact_inspect::inspect::inspect(
        &bytes,
        &options,
        &policy,
        &SessionConfig::hardened(),
    )
    .unwrap();
    let text = render_inspect(&report, &options, &policy);
    assert!(
        text.contains("singleton（chunk directory 不适用）"),
        "LFCP chunk 展开应标不适用：\n{text}"
    );
}

#[test]
fn lfsm_source_location_fields_expand_with_zero_based_rows() {
    let bytes = lfsm_with_source_locations(&[11, 12, 13]);
    let options = fields_options();
    let policy = default_policy();
    let report = laneflow_artifact_inspect::inspect::inspect(
        &bytes,
        &options,
        &policy,
        &SessionConfig::hardened(),
    )
    .unwrap();
    let text = render_inspect(&report, &options, &policy);
    assert!(
        text.contains("SourceModules.SourceLocation[0].startLine = 11"),
        "零基逻辑行与字段名缺失：\n{text}"
    );
    assert!(text.contains("SourceModules.SourceLocation[2].startLine = 13"));
    assert!(!text.contains("SourceModules.SourceLocation[3].startLine"));
}

#[test]
fn row_range_filter_limits_field_expansion() {
    let bytes = lfsm_with_source_locations(&[11, 12, 13, 14]);
    let options = fields_options();
    let policy = DisplayPolicy {
        filters: InspectFilters {
            rows: Some(RowRange { start: 1, end: 3 }),
            ..Default::default()
        },
        limits: ReportLimits::default(),
    };
    let report = laneflow_artifact_inspect::inspect::inspect(
        &bytes,
        &options,
        &policy,
        &SessionConfig::hardened(),
    )
    .unwrap();
    let text = render_inspect(&report, &options, &policy);
    assert!(text.contains("SourceLocation[1].startLine = 12"));
    assert!(text.contains("SourceLocation[2].startLine = 13"));
    assert!(!text.contains("SourceLocation[0].startLine"));
    assert!(!text.contains("SourceLocation[3].startLine"));
}

#[test]
fn field_expansion_reports_truncation_under_row_budget() {
    let lines: Vec<u32> = (0..100).collect();
    let bytes = lfsm_with_source_locations(&lines);
    let options = fields_options();
    let policy = DisplayPolicy {
        filters: InspectFilters::default(),
        limits: ReportLimits {
            max_expanded_rows: 10,
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
    // 预算 10 行先被首节 singleton 表消耗 1 行，SourceLocation 再展开 9 行。
    assert!(
        text.contains("因行数限额截断，剩余 91 行未展开"),
        "应明确报告行数限额截断：\n{text}"
    );
}

#[test]
fn section_filter_by_ordinal_and_name() {
    let bytes = lfsm_with_source_locations(&[11]);
    for selector in [
        SectionSelector::Ordinal(1),
        SectionSelector::Name("SourceModules".to_string()),
    ] {
        let options = InspectOptions {
            expand: ExpandSet {
                sections: true,
                ..Default::default()
            },
        };
        let policy = DisplayPolicy {
            filters: InspectFilters {
                section: Some(selector),
                ..Default::default()
            },
            limits: ReportLimits::default(),
        };
        let report = laneflow_artifact_inspect::inspect::inspect(
            &bytes,
            &options,
            &policy,
            &SessionConfig::hardened(),
        )
        .unwrap();
        assert_eq!(report.sections.len(), 1);
        assert_eq!(report.sections[0].identity.name, "SourceModules");
    }
}

#[test]
fn lfsd_embedded_member_row_expands_registered_bytes_only() {
    let bytes = lfsd_with_evidence_change("k", "locator-a", "locator-a");
    let options = fields_options();
    let policy = default_policy();
    let report = laneflow_artifact_inspect::inspect::inspect(
        &bytes,
        &options,
        &policy,
        &SessionConfig::hardened(),
    )
    .unwrap();
    let text = render_inspect(&report, &options, &policy);
    assert!(
        (text.contains("beforeValue{member=0}") || text.contains("afterValue{member=0}"))
            && text.contains("locator-a"),
        "已登记成员 Bytes 应展开内嵌行：\n{text}"
    );
}

#[test]
fn unknown_magic_and_short_input_are_rejected() {
    let error = laneflow_artifact_inspect::inspect::inspect(
        b"NOPE",
        &InspectOptions::default(),
        &default_policy(),
        &SessionConfig::hardened(),
    )
    .unwrap_err();
    assert!(matches!(error, InspectError::TooShortForMagic { .. }));

    let mut bytes = minimal_object(PortableObjectKind::CanonicalArtifact);
    bytes[0..4].copy_from_slice(b"XXXX");
    let error = laneflow_artifact_inspect::inspect::inspect(
        &bytes,
        &InspectOptions::default(),
        &default_policy(),
        &SessionConfig::hardened(),
    )
    .unwrap_err();
    assert!(matches!(error, InspectError::UnknownMagic { .. }));
}

#[test]
fn corrupted_object_fails_preflight_without_dump() {
    let mut bytes = lfsm_with_source_locations(&[11, 12]);
    // 破坏 chunk 数据区（目录之后）：摘要失配。
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    let error = laneflow_artifact_inspect::inspect::inspect(
        &bytes,
        &InspectOptions::default(),
        &default_policy(),
        &SessionConfig::hardened(),
    )
    .unwrap_err();
    assert!(matches!(error, InspectError::Preflight(_)));
}

#[test]
fn rendering_is_deterministic_for_fixed_input() {
    let bytes = lfsm_with_source_locations(&[11, 12, 13]);
    let options = fields_options();
    let policy = default_policy();
    let first = {
        let report = laneflow_artifact_inspect::inspect::inspect(
            &bytes,
            &options,
            &policy,
            &SessionConfig::hardened(),
        )
        .unwrap();
        render_inspect(&report, &options, &policy)
    };
    let second = {
        let report = laneflow_artifact_inspect::inspect::inspect(
            &bytes,
            &options,
            &policy,
            &SessionConfig::hardened(),
        )
        .unwrap();
        render_inspect(&report, &options, &policy)
    };
    assert_eq!(first, second, "同机重复渲染必须逐字节一致");
}
