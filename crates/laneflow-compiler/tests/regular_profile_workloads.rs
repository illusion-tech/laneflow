//! 常规档的候选工作负载回归（#1441）。
//!
//! 借用现有有限百万档走生产前端与发射管线；本测试验证负载与操作覆盖，不能替代新常规档的
//! 政策准入、逐维边界与预算拒绝资格。受控请求字节不等于进程内存，不采集墙钟性能。

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use laneflow_compiler::{
    CompilationOutput, CompileLimits, Compiler, PortableDiffBase, PortableEmissionProvenance,
    PortablePublicationCandidate, check_portable_candidate, check_portable_policy_diff,
    check_portable_policy_sources, emit_portable_candidate, emit_portable_candidate_to_staging,
};
use laneflow_format::{
    FormatLimits, RegistryCheckedFieldValue, ValueCheckedObjectView, preflight_object_values,
};
use laneflow_static_contract::{
    CanonicalNetworkUsage, EntityKind, PortableObjectKind, StableId128,
};

#[path = "support/regular_profile_workload.rs"]
mod workload;
use workload::{WorkloadShape, build_mixed_network};

// 仅供候选预算评估；生产操作仍明确使用百万档，不能把比较报告当作预算准入已实现。
const CANDIDATE_OPERATION_BYTES: u64 = 256 * 1024 * 1024;

struct StagingDirectory(PathBuf);

impl Drop for StagingDirectory {
    fn drop(&mut self) {
        // 候选先释放文件 backing；这里只移除本测试创建的空目录。
        let _ = fs::remove_dir(&self.0);
    }
}

struct Expected {
    name: &'static str,
    shape: WorkloadShape,
    usage: CanonicalNetworkUsage,
    lir_records: u64,
    source_bytes: u64,
    disjoint_source_bytes: u64,
    signal_states: u64,
    lane_geometries: u32,
}

const JOINT: WorkloadShape = WorkloadShape::for_stable_entities(10_000, 32, 65);

const CASES: [Expected; 8] = [
    Expected {
        name: "mixed",
        shape: WorkloadShape::for_stable_entities(10_000, 2, 2),
        usage: CanonicalNetworkUsage::new(10_000, 822, 399, 14_169, 2_682),
        lir_records: 28_904,
        source_bytes: 1_570_997,
        disjoint_source_bytes: 1_664_701,
        signal_states: 1_596,
        lane_geometries: 1_269,
    },
    Expected {
        name: "signal-dense",
        shape: WorkloadShape::for_stable_entities(10_000, 32, 2),
        usage: CanonicalNetworkUsage::new(10_000, 1_784, 55, 71_773, 3_918),
        lir_records: 90_974,
        source_bytes: 5_283_991,
        disjoint_source_bytes: 5_722_899,
        signal_states: 56_320,
        lane_geometries: 1_887,
    },
    Expected {
        name: "geometry-dense",
        shape: WorkloadShape::for_stable_entities(10_000, 2, 65),
        usage: CanonicalNetworkUsage::new(10_000, 822, 399, 14_169, 78_093),
        lir_records: 179_726,
        source_bytes: 2_475_929,
        disjoint_source_bytes: 2_569_633,
        signal_states: 1_596,
        lane_geometries: 1_269,
    },
    Expected {
        name: "joint",
        shape: JOINT,
        usage: CanonicalNetworkUsage::new(10_000, 1_784, 55, 71_773, 118_263),
        lir_records: 319_664,
        source_bytes: 6_656_131,
        disjoint_source_bytes: 7_095_039,
        signal_states: 56_320,
        lane_geometries: 1_887,
    },
    Expected {
        name: "partitioned",
        shape: WorkloadShape {
            source_partitions: 16,
            ..JOINT
        },
        usage: CanonicalNetworkUsage::new(10_000, 1_784, 55, 71_773, 118_263),
        lir_records: 319_664,
        source_bytes: 6_931_297,
        disjoint_source_bytes: 7_370_385,
        signal_states: 56_320,
        lane_geometries: 1_887,
    },
    Expected {
        name: "long-documents",
        shape: WorkloadShape {
            long_documents: true,
            ..JOINT
        },
        usage: CanonicalNetworkUsage::new(10_000, 1_784, 55, 71_773, 118_263),
        lir_records: 319_664,
        source_bytes: 6_658_140,
        disjoint_source_bytes: 7_097_048,
        signal_states: 56_320,
        lane_geometries: 1_887,
    },
    Expected {
        name: "curved",
        shape: WorkloadShape {
            curved: true,
            ..JOINT
        },
        usage: CanonicalNetworkUsage::new(10_000, 1_784, 55, 71_773, 128_511),
        lir_records: 335_600,
        source_bytes: 6_665_947,
        disjoint_source_bytes: 7_104_855,
        signal_states: 56_320,
        lane_geometries: 1_887,
    },
    Expected {
        name: "combined",
        shape: WorkloadShape {
            source_partitions: 16,
            long_documents: true,
            curved: true,
            ..JOINT
        },
        usage: CanonicalNetworkUsage::new(10_000, 1_784, 55, 71_773, 128_511),
        lir_records: 335_600,
        source_bytes: 6_957_942,
        disjoint_source_bytes: 7_397_030,
        signal_states: 56_320,
        lane_geometries: 1_887,
    },
];

fn compile_case(expected: &Expected, shape: WorkloadShape) -> CompilationOutput {
    let unit = build_mixed_network(shape, false);
    assert_eq!(unit.module_count(), shape.source_partitions as usize + 2);
    assert_eq!(
        unit.source_document_count(),
        shape.source_partitions as usize + 2
    );
    assert_eq!(
        unit.module_descriptors()
            .map(|module| module.imports().len())
            .sum::<usize>(),
        shape.source_partitions as usize + 1
    );
    let source_bytes: u64 = unit
        .source_document_descriptors()
        .map(|document| u64::from(document.source_record_byte_len()))
        .sum();
    assert_eq!(
        source_bytes,
        if shape.disjoint {
            expected.disjoint_source_bytes
        } else {
            expected.source_bytes
        }
    );
    let output = Compiler::new()
        .compile(unit)
        .unwrap_or_else(|diagnostics| panic!("{}: {diagnostics}", expected.name));
    assert_eq!(output.lir().canonical_network_usage(), expected.usage);
    assert_eq!(output.metrics().lir_record_count(), expected.lir_records);
    assert!(output.metrics().compiler_controlled_peak_bytes() <= CANDIDATE_OPERATION_BYTES);
    println!(
        "regular-profile case={} shape={shape:?} source_bytes={source_bytes} usage={:?} lir_records={} compile_peak={}",
        expected.name,
        expected.usage,
        expected.lir_records,
        output.metrics().compiler_controlled_peak_bytes()
    );
    output
}

fn artifact_view(candidate: &PortablePublicationCandidate) -> ValueCheckedObjectView<'_> {
    preflight_object_values(
        candidate.canonical_artifact().bytes(),
        PortableObjectKind::CanonicalArtifact,
        FormatLimits::HARD,
    )
    .unwrap()
}

fn identities(view: ValueCheckedObjectView<'_>) -> BTreeSet<StableId128> {
    view.registry_view()
        .section(1)
        .unwrap()
        .table(0)
        .unwrap()
        .rows()
        .map(|row| match row.field_by_tag(3).unwrap().value().unwrap() {
            RegistryCheckedFieldValue::StableId128(value) => value,
            _ => panic!("identity field type"),
        })
        .collect()
}

fn assert_role_coverage(candidate: &PortablePublicationCandidate, expected: &Expected) {
    let view = artifact_view(candidate).registry_view();
    let phase_ordinal = EntityKind::ALL
        .iter()
        .position(|kind| *kind == EntityKind::SignalPhase)
        .unwrap();
    let states: u64 = view
        .section(2)
        .unwrap()
        .table(phase_ordinal as u32)
        .unwrap()
        .rows()
        .map(|row| match row.field_by_tag(5).unwrap().value().unwrap() {
            RegistryCheckedFieldValue::RecordVector(values) => u64::from(values.len()),
            _ => panic!("signal states field type"),
        })
        .sum();
    assert_eq!(states, expected.signal_states, "role 19");
    let spatial = view.section(4).unwrap();
    assert_eq!(
        spatial.table(1).unwrap().row_count(),
        expected.lane_geometries,
        "role 28"
    );
    assert_eq!(spatial.table(2).unwrap().row_count(), 48, "role 29");
    assert_eq!(spatial.table(3).unwrap().row_count(), 12, "role 32");
}

fn diff_rows(candidate: &PortablePublicationCandidate, section: u32) -> u32 {
    preflight_object_values(
        candidate.semantic_diff().bytes(),
        PortableObjectKind::SemanticDiff,
        FormatLimits::HARD,
    )
    .unwrap()
    .registry_view()
    .section(section)
    .unwrap()
    .table(0)
    .unwrap()
    .row_count()
}

fn emit_and_check(
    expected: &Expected,
    operation: &str,
    output: &CompilationOutput,
    base: PortableDiffBase<'_>,
    staging: Option<&Path>,
) -> PortablePublicationCandidate {
    let provenance = PortableEmissionProvenance::try_new("regular-profile-candidate").unwrap();
    let candidate = match staging {
        Some(directory) => emit_portable_candidate_to_staging(
            output,
            &provenance,
            FormatLimits::HARD,
            base,
            directory,
        ),
        None => emit_portable_candidate(output, &provenance, FormatLimits::HARD, base),
    }
    .unwrap();
    let report = candidate.resource_report();
    assert!(report.peak_live_bytes() <= CANDIDATE_OPERATION_BYTES);
    assert_role_coverage(&candidate, expected);
    check_portable_policy_sources(
        candidate.canonical_artifact().bytes(),
        output.source_map_input(),
        candidate.source_map().bytes(),
        FormatLimits::HARD,
        &CompileLimits::single_network_1m(),
    )
    .unwrap();
    check_portable_policy_diff(
        base,
        candidate.canonical_artifact().bytes(),
        candidate.semantic_diff().bytes(),
        FormatLimits::HARD,
        &CompileLimits::single_network_1m(),
    )
    .unwrap();
    println!(
        "regular-profile case={} operation={operation} file_backed={} emission_peak={} diff_peak={} lfca_bytes={} lfsm_bytes={} lfsd_bytes={}",
        expected.name,
        staging.is_some(),
        report.peak_live_bytes(),
        report.diff_emission_peak_bytes(),
        candidate.canonical_artifact().byte_length().get(),
        candidate.source_map().byte_length().get(),
        candidate.semantic_diff().byte_length().get()
    );
    candidate
}

#[test]
fn regular_profile_candidates_cover_mixed_density_and_diff_operations() {
    let staging_directory = StagingDirectory(
        std::env::temp_dir().join(format!("laneflow-regular-profile-{}", std::process::id())),
    );
    let directory = &staging_directory.0;
    fs::create_dir(directory).unwrap();
    for expected in &CASES {
        let shape = expected.shape;
        let output = compile_case(expected, shape);
        let base = emit_and_check(
            expected,
            "genesis",
            &output,
            PortableDiffBase::Genesis,
            None,
        );
        let file = emit_and_check(
            expected,
            "genesis",
            &output,
            PortableDiffBase::Genesis,
            Some(directory),
        );
        assert_eq!(
            base.canonical_artifact().bytes(),
            file.canonical_artifact().bytes()
        );
        assert_eq!(base.source_map().bytes(), file.source_map().bytes());
        assert_eq!(base.semantic_diff().bytes(), file.semantic_diff().bytes());
        check_portable_candidate(file, FormatLimits::HARD).unwrap();
        let base_view = artifact_view(&base);
        let base_identities = identities(base_view);
        assert_eq!(base_identities.len(), 10_000);
        for staging in [None, Some(directory.as_path())] {
            let candidate = emit_and_check(
                expected,
                "self",
                &output,
                PortableDiffBase::Artifact(base_view),
                staging,
            );
            for section in 1..=6 {
                assert_eq!(diff_rows(&candidate, section), 0, "self diff");
            }
            check_portable_candidate(candidate, FormatLimits::HARD).unwrap();
        }
        if expected.name == "mixed" {
            let permuted = Compiler::new()
                .compile(build_mixed_network(shape, true))
                .unwrap();
            assert_eq!(permuted.lir().canonical_network_usage(), expected.usage);
            assert_eq!(
                permuted.metrics().semantic_fingerprint(),
                output.metrics().semantic_fingerprint()
            );
            let candidate = emit_and_check(
                expected,
                "source-permuted",
                &permuted,
                PortableDiffBase::Genesis,
                None,
            );
            assert_eq!(base.network_revision(), candidate.network_revision());
            assert_ne!(
                base.source_collection_digest(),
                candidate.source_collection_digest()
            );
            check_portable_candidate(candidate, FormatLimits::HARD).unwrap();
        }
        drop(output);
        for (operation, variant) in [
            (
                "changed",
                WorkloadShape {
                    changed: true,
                    ..shape
                },
            ),
            (
                "reordered",
                WorkloadShape {
                    reordered: true,
                    ..shape
                },
            ),
            (
                "disjoint",
                WorkloadShape {
                    disjoint: true,
                    ..shape
                },
            ),
        ] {
            let output = compile_case(expected, variant);
            for staging in [None, Some(directory.as_path())] {
                let candidate = emit_and_check(
                    expected,
                    operation,
                    &output,
                    PortableDiffBase::Artifact(base_view),
                    staging,
                );
                let target_identities = identities(artifact_view(&candidate));
                if variant.disjoint {
                    assert!(base_identities.is_disjoint(&target_identities));
                    assert_eq!(
                        diff_rows(&candidate, 1),
                        20_000,
                        "all entities removed and added"
                    );
                } else {
                    assert_eq!(base_identities, target_identities);
                    assert_ne!(base.network_revision(), candidate.network_revision());
                    if variant.changed {
                        assert_eq!(
                            diff_rows(&candidate, 1),
                            shape.synthetic_tiles,
                            "entry speed modifications"
                        );
                        let changes = preflight_object_values(
                            candidate.semantic_diff().bytes(),
                            PortableObjectKind::SemanticDiff,
                            FormatLimits::HARD,
                        )
                        .unwrap();
                        for row in changes
                            .registry_view()
                            .section(1)
                            .unwrap()
                            .table(0)
                            .unwrap()
                            .rows()
                        {
                            assert!(
                                matches!(
                                    row.field_by_tag(1).unwrap().value().unwrap(),
                                    RegistryCheckedFieldValue::U8(2)
                                ),
                                "only entity modifications"
                            );
                        }
                        assert!(diff_rows(&candidate, 3) > 0, "geometry changes");
                        assert!(diff_rows(&candidate, 4) > 0, "static rule changes");
                    } else {
                        assert_eq!(diff_rows(&candidate, 1), 0, "entity values preserved");
                        assert!(diff_rows(&candidate, 2) > 0, "ordered relation changes");
                        assert_eq!(diff_rows(&candidate, 3), 0, "geometry preserved");
                    }
                }
                check_portable_candidate(candidate, FormatLimits::HARD).unwrap();
            }
        }
        check_portable_candidate(base, FormatLimits::HARD).unwrap();
        assert_eq!(
            fs::read_dir(directory).unwrap().count(),
            0,
            "staging released"
        );
    }
    fs::remove_dir(directory).unwrap();
}

#[test]
fn regular_profile_source_metadata_preserves_canonical_usage_and_revision() {
    let ordinary = &CASES[3];
    let long_documents = &CASES[5];
    let output = compile_case(ordinary, ordinary.shape);
    let longer = compile_case(long_documents, long_documents.shape);
    assert_eq!(
        output.metrics().semantic_fingerprint(),
        longer.metrics().semantic_fingerprint()
    );
    let base = emit_and_check(
        ordinary,
        "metadata-base",
        &output,
        PortableDiffBase::Genesis,
        None,
    );
    let changed = emit_and_check(
        long_documents,
        "long-documents",
        &longer,
        PortableDiffBase::Genesis,
        None,
    );
    assert_eq!(base.network_revision(), changed.network_revision());
    assert_ne!(
        base.source_collection_digest(),
        changed.source_collection_digest()
    );
    assert!(changed.source_map().byte_length().get() > base.source_map().byte_length().get());
    assert_eq!(
        identities(artifact_view(&base)),
        identities(artifact_view(&changed))
    );

    let provenance = PortableEmissionProvenance::try_new("p".repeat(128)).unwrap();
    let different_provenance = emit_portable_candidate(
        &output,
        &provenance,
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .unwrap();
    assert!(different_provenance.resource_report().peak_live_bytes() <= CANDIDATE_OPERATION_BYTES);
    assert_eq!(
        base.network_revision(),
        different_provenance.network_revision()
    );
    assert_eq!(
        base.source_collection_digest(),
        different_provenance.source_collection_digest()
    );
    assert!(
        different_provenance
            .canonical_artifact()
            .byte_length()
            .get()
            > base.canonical_artifact().byte_length().get()
    );
    assert_role_coverage(&different_provenance, ordinary);
    for candidate in [base, changed, different_provenance] {
        check_portable_candidate(candidate, FormatLimits::HARD).unwrap();
    }
}

#[test]
fn road_editing_document_key_boundary_precedes_profile_admission() {
    use laneflow_compiler::{SourceTextViolation, road_editing::RoadEditingModuleInput};
    let document = "d".repeat(53);
    assert!(RoadEditingModuleInput::try_new(&document, &[], None).is_ok());
    let overlong = "d".repeat(54);
    let error = RoadEditingModuleInput::try_new(&overlong, &[], None).unwrap_err();
    assert_eq!(
        error.violation(),
        SourceTextViolation::TooLong {
            limit: 53,
            observed: 54
        }
    );
}
