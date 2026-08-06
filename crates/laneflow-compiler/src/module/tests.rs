use std::sync::Arc;

use crate::declaration::TypedAstDeclaration;
use crate::*;

use super::*;
use crate::{DiagnosticCode, DiagnosticPayload, LaneEdgeReference, SourceModuleHeaderInput};

fn header(namespace: &str, document: &str) -> SourceModuleHeader {
    SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: namespace,
            source_document_key: document,
            generator_build_id: "git:0123456789abcdef",
            parameters_and_inputs_digest: [0x11; 32],
            frontend_options_digest: [0x22; 32],
            random_seed: Some(42),
            provenance: "repository:laneflow",
        },
        &CompileLimits::p100_initial_v1(),
    )
    .unwrap()
}

fn module(namespace: &str, imports: &[&str]) -> SyntheticModule {
    module_with_document(namespace, namespace, imports)
}

fn module_with_document(namespace: &str, document: &str, imports: &[&str]) -> SyntheticModule {
    let limits = CompileLimits::p100_initial_v1();
    let mut builder = SyntheticModuleBuilder::new(header(namespace, document), &limits).unwrap();
    for import in imports {
        builder.add_import(import).unwrap();
    }
    builder.finish().unwrap()
}

fn expect_diagnostics<T>(result: Result<T, DiagnosticBundle>) -> DiagnosticBundle {
    match result {
        Ok(_) => panic!("expected structured diagnostics"),
        Err(bundle) => bundle,
    }
}

fn add_lane_edge_at<'a>(
    builder: &'a mut SyntheticModuleBuilder,
    input: LaneEdgeInput<'_>,
    line: u32,
) -> Result<&'a mut SyntheticModuleBuilder, DiagnosticBundle> {
    builder.add_lane_edge_at(input, SourceSpan::point(Arc::from("source.test"), line, 1))
}

#[test]
fn document_digest_and_length_are_bound_to_the_module() {
    let module = module("city/a", &["city/base"]);
    assert_eq!(
        module.descriptor().source_language(),
        SourceLanguage::SyntheticDsl
    );
    assert_eq!(module.descriptor().frontend_version(), 1);
    let document = module.source_documents().next().unwrap();
    assert_eq!(
        u64::from(document.source_record_byte_len()),
        module.admitted.resource_counts.source_bytes
    );
    assert_ne!(
        module.descriptor().source_document_set_digest(),
        document.source_document_digest()
    );
    assert_eq!(
        module.descriptor().imports().collect::<Vec<_>>(),
        ["city/base"]
    );
}

#[test]
fn empty_module_source_record_has_a_position_independent_known_vector() {
    let source_document_key = Arc::from("source.known-vector");
    let fixed_header = SourceModuleHeader {
        authoring_namespace_id: Arc::from("city/known-vector"),
        source_document_key: Arc::clone(&source_document_key),
        generator_build_id: Arc::from("git:0123456789abcdef"),
        parameters_and_inputs_digest: [0x11; 32],
        frontend_options_digest: [0x22; 32],
        random_seed: Some(42),
        provenance: Arc::from("repository:laneflow"),
        declaration_span: SourceSpan::point(source_document_key, 7, 11),
    };
    let module = SyntheticModuleBuilder::new(fixed_header, &CompileLimits::p100_initial_v1())
        .unwrap()
        .finish()
        .unwrap();

    let document = module.source_documents().next().unwrap();
    assert_eq!(document.source_record_byte_len(), 202);
    assert_eq!(
        *document.source_document_digest(),
        [
            0x30, 0x24, 0xae, 0x9e, 0xb4, 0xa2, 0xcd, 0x16, 0x59, 0x3b, 0xd9, 0x7a, 0xf3, 0xbe,
            0x54, 0xcb, 0x06, 0x61, 0x8d, 0xce, 0x2e, 0x24, 0x3a, 0xc7, 0xc1, 0xb2, 0x3a, 0x12,
            0xef, 0x02, 0xa3, 0x4a,
        ]
    );
    assert_eq!(
        *module.descriptor().source_document_set_digest(),
        [
            0xe2, 0x3b, 0x2c, 0xd1, 0x0c, 0xc8, 0xf7, 0x4c, 0x1f, 0x21, 0x28, 0x03, 0x73, 0xcf,
            0xff, 0x89, 0x16, 0x3f, 0x63, 0x00, 0x6f, 0x26, 0x2f, 0x63, 0x15, 0x06, 0xee, 0x4f,
            0xad, 0x2f, 0xcf, 0x7e,
        ]
    );
}

#[test]
fn lane_edge_accepts_terminal_and_self_loop_topology() {
    let limits = CompileLimits::p100_initial_v1();
    let mut builder =
        SyntheticModuleBuilder::new(header("city/a", "source.test"), &limits).unwrap();
    builder
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "terminal",
            length_meters: 10.0,
            speed_limit_meters_per_second: 13.0,
            successors: &[],
        })
        .unwrap();
    add_lane_edge_at(
        &mut builder,
        LaneEdgeInput {
            lane_edge_key: "loop",
            length_meters: 20.0,
            speed_limit_meters_per_second: 8.0,
            successors: &[LaneEdgeReference::local("loop")],
        },
        20,
    )
    .unwrap();

    let module = builder.finish().unwrap();
    assert_eq!(module.admitted.resource_counts.declaration_count, 2);
    let TypedAstDeclaration::LaneEdge(terminal) = &module.admitted.declarations[0] else {
        panic!("expected LaneEdge declaration")
    };
    assert!(terminal.successors.is_empty());
    assert_eq!(terminal.header.span.source_document_key(), "source.test");
    let TypedAstDeclaration::LaneEdge(loop_edge) = &module.admitted.declarations[1] else {
        panic!("expected LaneEdge declaration")
    };
    assert_eq!(loop_edge.successors.len(), 1);
    assert_eq!(loop_edge.successors[0].declaration_key.as_ref(), "loop");
}

#[test]
fn lane_edge_rejects_non_finite_and_non_positive_scalars_without_mutation() {
    let limits = CompileLimits::p100_initial_v1();
    for (length, speed, expected_code) in [
        (f64::NAN, 1.0, DiagnosticCode::InvalidLaneEdgeLength),
        (f64::INFINITY, 1.0, DiagnosticCode::InvalidLaneEdgeLength),
        (0.0, 1.0, DiagnosticCode::InvalidLaneEdgeLength),
        (1.0e-9, 1.0, DiagnosticCode::InvalidLaneEdgeLength),
        (
            1.0,
            f64::NEG_INFINITY,
            DiagnosticCode::InvalidLaneEdgeSpeedLimit,
        ),
        (1.0, 0.0, DiagnosticCode::InvalidLaneEdgeSpeedLimit),
    ] {
        let mut builder =
            SyntheticModuleBuilder::new(header("city/a", "source.test"), &limits).unwrap();
        let failure = expect_diagnostics(add_lane_edge_at(
            &mut builder,
            LaneEdgeInput {
                lane_edge_key: "edge-a",
                length_meters: length,
                speed_limit_meters_per_second: speed,
                successors: &[],
            },
            10,
        ));
        assert_eq!(failure.diagnostics()[0].code(), expected_code);
        let module = builder.finish().unwrap();
        assert_eq!(module.admitted.resource_counts.declaration_count, 0);
    }
}

#[test]
fn lane_edge_requires_explicit_import_and_valid_reference_tokens() {
    let limits = CompileLimits::p100_initial_v1();
    let mut builder =
        SyntheticModuleBuilder::new(header("city/a", "source.test"), &limits).unwrap();
    let missing_import = expect_diagnostics(add_lane_edge_at(
        &mut builder,
        LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 1.0,
            speed_limit_meters_per_second: 1.0,
            successors: &[LaneEdgeReference::imported("city/base", "edge-b")],
        },
        10,
    ));
    assert_eq!(
        missing_import.diagnostics()[0].code(),
        DiagnosticCode::UnimportedReferenceModule
    );

    let invalid_namespace = expect_diagnostics(add_lane_edge_at(
        &mut builder,
        LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 1.0,
            speed_limit_meters_per_second: 1.0,
            successors: &[LaneEdgeReference::imported("city base", "edge-b")],
        },
        11,
    ));
    assert_eq!(
        invalid_namespace.diagnostics()[0].code(),
        DiagnosticCode::InvalidReferenceNamespace
    );

    let invalid_key = expect_diagnostics(add_lane_edge_at(
        &mut builder,
        LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 1.0,
            speed_limit_meters_per_second: 1.0,
            successors: &[LaneEdgeReference::local("edge b")],
        },
        12,
    ));
    assert_eq!(
        invalid_key.diagnostics()[0].code(),
        DiagnosticCode::InvalidReferenceKey
    );

    builder.add_import("city/base").unwrap();
    add_lane_edge_at(
        &mut builder,
        LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 1.0,
            speed_limit_meters_per_second: 1.0,
            successors: &[LaneEdgeReference::imported("city/base", "edge-b")],
        },
        13,
    )
    .unwrap();
}

#[test]
fn duplicate_lane_edge_and_successor_fail_without_mutating_prior_state() {
    let limits = CompileLimits::p100_initial_v1();
    let mut builder =
        SyntheticModuleBuilder::new(header("city/a", "source.test"), &limits).unwrap();
    let duplicate_successor = expect_diagnostics(add_lane_edge_at(
        &mut builder,
        LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 1.0,
            speed_limit_meters_per_second: 1.0,
            successors: &[
                LaneEdgeReference::local("edge-b"),
                LaneEdgeReference::imported("city/a", "edge-b"),
            ],
        },
        10,
    ));
    assert_eq!(
        duplicate_successor.diagnostics()[0].code(),
        DiagnosticCode::DuplicateLaneEdgeSuccessor
    );

    add_lane_edge_at(
        &mut builder,
        LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 1.0,
            speed_limit_meters_per_second: 1.0,
            successors: &[],
        },
        20,
    )
    .unwrap();
    let duplicate_declaration = expect_diagnostics(add_lane_edge_at(
        &mut builder,
        LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 2.0,
            speed_limit_meters_per_second: 2.0,
            successors: &[],
        },
        30,
    ));
    assert_eq!(
        duplicate_declaration.diagnostics()[0].code(),
        DiagnosticCode::DuplicateDeclaration
    );
    assert_eq!(
        duplicate_declaration.diagnostics()[0].related_spans().len(),
        1
    );

    let module = builder.finish().unwrap();
    assert_eq!(module.admitted.resource_counts.declaration_count, 1);
    let TypedAstDeclaration::LaneEdge(edge) = &module.admitted.declarations[0] else {
        panic!("expected LaneEdge declaration")
    };
    assert_eq!(edge.length.value(), 1.0);
}

#[test]
fn lane_edge_successor_order_is_not_source_identity() {
    let limits = CompileLimits::p100_initial_v1();
    let left_successors = [
        LaneEdgeReference::local("edge-c"),
        LaneEdgeReference::local("edge-b"),
    ];
    let right_successors = [
        LaneEdgeReference::local("edge-b"),
        LaneEdgeReference::local("edge-c"),
    ];
    let build = |successors: &[LaneEdgeReference<'_>]| {
        let mut builder =
            SyntheticModuleBuilder::new(header("city/a", "source.test"), &limits).unwrap();
        add_lane_edge_at(
            &mut builder,
            LaneEdgeInput {
                lane_edge_key: "edge-a",
                length_meters: 12.5,
                speed_limit_meters_per_second: 13.75,
                successors,
            },
            10,
        )
        .unwrap();
        builder.finish().unwrap()
    };

    let left = build(&left_successors);
    let right = build(&right_successors);
    assert_eq!(
        left.source_documents()
            .next()
            .unwrap()
            .source_document_digest(),
        right
            .source_documents()
            .next()
            .unwrap()
            .source_document_digest()
    );
}

#[test]
fn lane_edge_source_record_has_a_known_vector() {
    let source_document_key = Arc::from("source.lane-edge-vector");
    let fixed_header = SourceModuleHeader {
        authoring_namespace_id: Arc::from("city/lane-edge-vector"),
        source_document_key: Arc::clone(&source_document_key),
        generator_build_id: Arc::from("git:0123456789abcdef"),
        parameters_and_inputs_digest: [0x11; 32],
        frontend_options_digest: [0x22; 32],
        random_seed: Some(42),
        provenance: Arc::from("repository:laneflow"),
        declaration_span: SourceSpan::point(Arc::clone(&source_document_key), 7, 11),
    };
    let limits = CompileLimits::p100_initial_v1();
    let mut builder = SyntheticModuleBuilder::new(fixed_header, &limits).unwrap();
    builder
        .add_lane_edge_at(
            LaneEdgeInput {
                lane_edge_key: "edge-a",
                length_meters: 12.5,
                speed_limit_meters_per_second: 13.75,
                successors: &[
                    LaneEdgeReference::local("edge-c"),
                    LaneEdgeReference::local("edge-b"),
                ],
            },
            SourceSpan::point(source_document_key, 13, 17),
        )
        .unwrap();
    let module = builder.finish().unwrap();

    let document = module.source_documents().next().unwrap();
    assert_eq!(document.source_record_byte_len(), 360);
    assert_eq!(
        *document.source_document_digest(),
        [
            0xc9, 0x99, 0xb7, 0xae, 0x09, 0x12, 0xf4, 0x05, 0x31, 0x15, 0xfc, 0xbf, 0x3e, 0x59,
            0xa2, 0xa9, 0x85, 0xb4, 0xb4, 0x60, 0x42, 0x63, 0x13, 0xb2, 0xc4, 0xe2, 0x81, 0x7d,
            0xc7, 0xbc, 0x1b, 0x3c,
        ]
    );
}

#[test]
fn lane_edge_counters_follow_the_calibrated_record_formula() {
    let limits = CompileLimits::p100_initial_v1();
    let mut builder =
        SyntheticModuleBuilder::new(header("city/a", "source.test"), &limits).unwrap();
    builder.add_import("city/base").unwrap();
    add_lane_edge_at(
        &mut builder,
        LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 12.0,
            speed_limit_meters_per_second: 8.0,
            successors: &[
                LaneEdgeReference::local("edge-b"),
                LaneEdgeReference::imported("city/base", "edge-c"),
            ],
        },
        10,
    )
    .unwrap();
    let module = builder.finish().unwrap();

    let counts = &module.admitted.resource_counts;
    assert_eq!(counts.declaration_count, 1);
    assert_eq!(counts.reference_count, 2);
    assert_eq!(counts.relation_occurrence_count, 2);
    assert_eq!(counts.identity_field_occurrence_count, 2);
    assert_eq!(counts.symbol_count, 1);
    assert_eq!(counts.typed_ast_record_count, 9);
    assert_eq!(counts.string_item_count, 7);
    assert_eq!(
        u64::from(
            module
                .source_documents()
                .next()
                .unwrap()
                .source_record_byte_len()
        ),
        counts.source_bytes
    );
}

#[test]
fn facility_kind_validation_matches_the_accepted_seed_and_extension_prefixes() {
    let limits = CompileLimits::p100_initial_v1();
    let mut builder =
        SyntheticModuleBuilder::new(header("city/a", "source.test"), &limits).unwrap();
    for (key, kind_id) in [
        ("band-sidewalk", "sidewalk"),
        ("band-median", "median"),
        ("band-planting", "plantingStrip"),
        ("band-facility", "facilityStrip"),
        ("band-shoulder", "shoulder"),
        ("band-custom", "x-platform"),
    ] {
        builder
            .add_facility_band(FacilityBandInput {
                facility_band_key: key,
                kind_id,
            })
            .unwrap();
    }

    for invalid in ["x-", "x-lane-"] {
        let diagnostics = expect_diagnostics(builder.add_facility_band(FacilityBandInput {
            facility_band_key: "invalid-band",
            kind_id: invalid,
        }));
        assert_eq!(
            diagnostics.diagnostics()[0].code(),
            DiagnosticCode::InvalidFacilityKind
        );
    }

    let module = builder.finish().unwrap();
    assert_eq!(module.admitted.resource_counts.declaration_count, 6);
}

#[test]
fn duplicate_and_self_imports_fail_with_source_context() {
    let limits = CompileLimits::p100_initial_v1();
    let mut builder = SyntheticModuleBuilder::new(header("city/a", "source.a"), &limits).unwrap();
    builder.add_import("city/base").unwrap();
    let duplicate = expect_diagnostics(builder.add_import("city/base"));
    assert_eq!(
        duplicate.diagnostics()[0].code(),
        DiagnosticCode::DuplicateImport
    );
    assert!(duplicate.diagnostics()[0].primary_span().is_some());
    assert_eq!(duplicate.diagnostics()[0].related_spans().len(), 1);

    let self_import = expect_diagnostics(builder.add_import("city/a"));
    assert_eq!(
        self_import.diagnostics()[0].code(),
        DiagnosticCode::ImportCycle
    );
}

#[test]
fn import_limit_failure_does_not_mutate_the_module() {
    let limits = CompileLimits::p100_initial_v1();
    let mut builder = SyntheticModuleBuilder::new(header("city/a", "source.a"), &limits).unwrap();
    let import_limit = limits.value(CompileLimitDimension::ImportEdgeCount);
    for index in 0..import_limit {
        builder
            .add_import(&format!("city/import/{index:04}"))
            .unwrap();
    }

    let failure = expect_diagnostics(builder.add_import("city/import/overflow"));
    assert!(matches!(
        failure.diagnostics()[0].payload(),
        DiagnosticPayload::CompileLimitExceeded {
            dimension: CompileLimitDimension::ImportEdgeCount,
            limit,
            observed,
        } if *limit == import_limit && *observed == import_limit + 1
    ));
    let module = builder.finish().unwrap();
    assert_eq!(
        u64::try_from(module.descriptor().imports().len()).unwrap(),
        import_limit
    );
}

#[test]
fn source_record_encoder_fails_before_over_limit_allocation() {
    let header = header("city/a", "source.a");
    let expected_len = encoded_source_record_len(&header, &[], &[]).unwrap();
    let failure = expect_diagnostics(encode_source_record(&header, &[], &[], expected_len - 1));
    assert!(matches!(
        failure.diagnostics()[0].payload(),
        DiagnosticPayload::CompileLimitExceeded {
            dimension: CompileLimitDimension::SourceBytesPerModule,
            limit,
            observed,
        } if *limit == expected_len - 1 && *observed == expected_len
    ));
}

#[test]
fn compilation_unit_rejects_unknown_imports_and_duplicate_namespaces() {
    let limits = CompileLimits::p100_initial_v1();
    let mut builder = CompilationUnitBuilder::new(limits.clone());
    builder
        .add_synthetic_module(module("city/a", &["city/missing"]))
        .unwrap();
    let unknown = expect_diagnostics(builder.build());
    assert!(matches!(
        unknown.diagnostics()[0].payload(),
        DiagnosticPayload::UnknownImport { namespace } if namespace.as_ref() == "city/missing"
    ));

    let mut builder = CompilationUnitBuilder::new(limits);
    builder.add_synthetic_module(module("city/a", &[])).unwrap();
    let duplicate = expect_diagnostics(builder.add_synthetic_module(module("city/a", &[])));
    assert_eq!(
        duplicate.diagnostics()[0].code(),
        DiagnosticCode::DuplicateModuleNamespace
    );
    assert_eq!(duplicate.diagnostics()[0].related_spans().len(), 1);
}

#[test]
fn compilation_unit_rejects_duplicate_source_document_keys_atomically() {
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    builder
        .add_synthetic_module(module_with_document("city/a", "shared.document", &[]))
        .unwrap();
    let duplicate = expect_diagnostics(builder.add_synthetic_module(module_with_document(
        "city/b",
        "shared.document",
        &[],
    )));
    assert!(matches!(
        duplicate.diagnostics()[0].payload(),
        DiagnosticPayload::DuplicateSourceDocumentKey {
            source_document_key
        } if source_document_key.as_ref() == "shared.document"
    ));
    assert_eq!(duplicate.diagnostics()[0].related_spans().len(), 1);

    // 重复文档失败发生在任何累计计数变更之前；修正文档键后，同一 namespace 可直接
    // 重试。这里同时防止未来把文档唯一性检查移到非原子的 build 阶段。
    builder
        .add_synthetic_module(module_with_document("city/b", "city-b.document", &[]))
        .unwrap();
    let unit = builder.build().unwrap();
    assert_eq!(unit.module_descriptors().len(), 2);
}

#[test]
fn compilation_unit_uses_dependency_first_canonical_order() {
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    builder
        .add_synthetic_module(module("city/z", &["city/a"]))
        .unwrap();
    builder.add_synthetic_module(module("city/b", &[])).unwrap();
    builder.add_synthetic_module(module("city/a", &[])).unwrap();
    let unit = builder.build().unwrap();
    assert_eq!(
        unit.module_descriptors()
            .map(SourceModuleDescriptor::authoring_namespace_id)
            .collect::<Vec<_>>(),
        ["city/a", "city/b", "city/z"]
    );
}

#[test]
fn compilation_unit_reports_canonical_cycle() {
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    builder
        .add_synthetic_module(module("city/c", &["city/a"]))
        .unwrap();
    builder
        .add_synthetic_module(module("city/a", &["city/b"]))
        .unwrap();
    builder
        .add_synthetic_module(module("city/b", &["city/c"]))
        .unwrap();
    let bundle = expect_diagnostics(builder.build());
    assert!(matches!(
        bundle.diagnostics()[0].payload(),
        DiagnosticPayload::ImportCycle { namespaces }
            if namespaces.iter().map(AsRef::as_ref).collect::<Vec<&str>>()
                == ["city/a", "city/b", "city/c"]
    ));
}

#[test]
fn compilation_unit_reports_every_disjoint_cycle_in_canonical_order() {
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    for module in [
        module("city/d", &["city/c"]),
        module("city/a", &["city/b"]),
        module("city/c", &["city/d"]),
        module("city/b", &["city/a"]),
    ] {
        builder.add_synthetic_module(module).unwrap();
    }

    let bundle = expect_diagnostics(builder.build());
    assert_eq!(bundle.diagnostics().len(), 2);
    let cycles: Vec<_> = bundle
        .diagnostics()
        .iter()
        .map(|diagnostic| match diagnostic.payload() {
            DiagnosticPayload::ImportCycle { namespaces } => {
                namespaces.iter().map(AsRef::as_ref).collect::<Vec<&str>>()
            }
            other => panic!("expected import cycle, got {other:?}"),
        })
        .collect();
    assert_eq!(cycles, [["city/a", "city/b"], ["city/c", "city/d"]]);
    assert!(
        bundle
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.primary_span().is_some())
    );
}

#[test]
fn v1_rejects_a_multi_document_official_module_without_mutation() {
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    let test_module = TestOfficialModule::from_synthetic_with_documents(
        module_with_document("city/a", "source.primary", &[]),
        &[("source.secondary", b"secondary")],
    );
    let diagnostics = expect_diagnostics(builder.add_test_official_module(test_module));
    assert!(matches!(
        diagnostics.diagnostics()[0].payload(),
        DiagnosticPayload::CompileProfileIncompatible {
            profile_id,
            required_dimension: CompileLimitDimension::SourceDocumentCount,
        } if profile_id.as_ref() == "LF-COMP-P100-INITIAL-v1"
    ));

    builder
        .add_synthetic_module(module_with_document("city/a", "source.primary", &[]))
        .unwrap();
    assert_eq!(builder.build().unwrap().source_document_count(), 1);
}

#[test]
fn common_admission_rejects_duplicate_keys_inside_one_official_module_atomically() {
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v2());
    let duplicate = TestOfficialModule::from_synthetic_with_documents(
        module_with_document("city/a", "source/shared", &[]),
        &[("source/shared", b"duplicate")],
    );
    let diagnostics = expect_diagnostics(builder.add_test_official_module(duplicate));
    assert!(matches!(
        diagnostics.diagnostics()[0].payload(),
        DiagnosticPayload::DuplicateSourceDocumentKey { source_document_key }
            if source_document_key.as_ref() == "source/shared"
    ));

    builder
        .add_synthetic_module(module_with_document("city/a", "source/shared", &[]))
        .unwrap();
    assert_eq!(builder.build().unwrap().source_document_count(), 1);
}

#[test]
fn common_admission_enforces_canonical_document_order_before_index_commit() {
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v2());
    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        TestOfficialModule::from_synthetic_with_unsorted_documents(
            module_with_document("city/a", "source/a", &[]),
            &[("source/a", b"duplicate"), ("source/b", b"b")],
        )
    }));
    let panic = match unwind {
        Ok(_) => panic!("unsorted official documents must violate admission invariant"),
        Err(panic) => panic,
    };
    let message = panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or_default();
    assert!(message.contains("canonically sort source documents"));

    builder
        .add_synthetic_module(module_with_document("city/a", "source/a", &[]))
        .unwrap();
    assert_eq!(builder.build().unwrap().source_document_count(), 1);
}

#[test]
fn common_admission_enforces_every_owned_cumulative_resource_dimension_atomically() {
    let dimensions = [
        CompileLimitDimension::ModuleCount,
        CompileLimitDimension::SourceDocumentCount,
        CompileLimitDimension::ImportEdgeCount,
        CompileLimitDimension::SourceBytesTotal,
        CompileLimitDimension::DeclarationCount,
        CompileLimitDimension::TypedAstRecordCount,
        CompileLimitDimension::ReferenceCount,
        CompileLimitDimension::RelationOccurrenceCount,
        CompileLimitDimension::IdentityFieldOccurrenceCount,
        CompileLimitDimension::SymbolCount,
        CompileLimitDimension::StringItemCount,
        CompileLimitDimension::TotalStringBytes,
        CompileLimitDimension::ManeuverGateCount,
        CompileLimitDimension::WaitingZoneCount,
        CompileLimitDimension::RouteOccurrenceCount,
        CompileLimitDimension::GeometryPointCount,
        CompileLimitDimension::CompilerControlledLiveBytes,
    ];

    for dimension in dimensions {
        let limits = CompileLimits::p100_initial_v2().with_test_admission_limit(dimension, 0);
        let mut builder = CompilationUnitBuilder::new(limits);
        let diagnostics = match dimension {
            CompileLimitDimension::ImportEdgeCount => {
                let frontend_limits = CompileLimits::p100_initial_v1();
                let mut module_builder =
                    SyntheticModuleBuilder::new(header("city/a", "source/a"), &frontend_limits)
                        .unwrap();
                module_builder.add_import("city/target").unwrap();
                expect_diagnostics(builder.add_synthetic_module(module_builder.finish().unwrap()))
            }
            CompileLimitDimension::DeclarationCount
            | CompileLimitDimension::TypedAstRecordCount
            | CompileLimitDimension::ReferenceCount
            | CompileLimitDimension::RelationOccurrenceCount
            | CompileLimitDimension::IdentityFieldOccurrenceCount
            | CompileLimitDimension::SymbolCount
            | CompileLimitDimension::StringItemCount
            | CompileLimitDimension::TotalStringBytes
            | CompileLimitDimension::ManeuverGateCount
            | CompileLimitDimension::WaitingZoneCount
            | CompileLimitDimension::RouteOccurrenceCount
            | CompileLimitDimension::GeometryPointCount => {
                let mut module = TestOfficialModule::from_synthetic_with_documents(
                    module_with_document("city/a", "source/a", &[]),
                    &[],
                );
                module.force_resource_count(dimension, 1);
                expect_diagnostics(builder.add_test_official_module(module))
            }
            _ => expect_diagnostics(builder.add_synthetic_module(module_with_document(
                "city/a",
                "source/a",
                &[],
            ))),
        };
        assert!(matches!(
            diagnostics.diagnostics()[0].payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: actual_dimension,
                limit: 0,
                observed,
            } if *actual_dimension == dimension && *observed > 0
        ));
        assert!(
            builder.modules.is_empty(),
            "failed dimension: {dimension:?}"
        );
        assert!(builder.module_index.is_empty());
        assert!(builder.source_document_index_is_empty());
        assert_eq!(builder.totals, Default::default());
    }
}

#[test]
fn admission_sizing_accounts_for_builder_indexes_wrappers_and_result_modules() {
    let limits = CompileLimits::p100_initial_v2();
    let mut builder = CompilationUnitBuilder::new(limits.clone());
    builder
        .add_synthetic_module(module_with_document("city/a", "source/a", &[]))
        .unwrap();

    let sizing = AdmissionSizing::from_totals(
        builder.totals,
        limits.value(CompileLimitDimension::DiagnosticCount),
    );
    let expected_document_index = source_document_index_requested_bytes(1);
    let expected_module_index = requested_hash_table_bytes::<Arc<str>, usize>(1);
    assert_eq!(
        sizing.builder_live_bytes,
        builder
            .totals
            .module_payload_live_bytes
            .saturating_add(expected_document_index)
            .saturating_add(expected_module_index)
            .saturating_add(size_bytes::<AdmittedOfficialModule>(1))
    );
    assert_eq!(
        sizing.result_live_bytes,
        builder
            .totals
            .module_payload_live_bytes
            .saturating_add(expected_document_index)
            .saturating_add(size_bytes::<TypedAstModule>(1))
    );
    assert!(sizing.build_scratch_bytes > 0);
    assert!(sizing.build_peak_live_bytes > sizing.builder_live_bytes);

    let unit = builder.build().unwrap();
    assert_eq!(unit.controlled_live_bytes, sizing.result_live_bytes);
}

#[test]
fn build_checks_admission_scratch_and_peak_boundaries_before_freezing() {
    fn sizing_for_one_module() -> AdmissionSizing {
        let limits = CompileLimits::p100_initial_v2();
        let mut builder = CompilationUnitBuilder::new(limits.clone());
        builder
            .add_synthetic_module(module_with_document("city/a", "source/a", &[]))
            .unwrap();
        AdmissionSizing::from_totals(
            builder.totals,
            limits.value(CompileLimitDimension::DiagnosticCount),
        )
    }

    fn build_with_limit(
        dimension: CompileLimitDimension,
        limit: u32,
    ) -> Result<CompilationUnit, DiagnosticBundle> {
        let limits = CompileLimits::p100_initial_v2().with_test_admission_limit(dimension, limit);
        let mut builder = CompilationUnitBuilder::new(limits);
        builder
            .add_synthetic_module(module_with_document("city/a", "source/a", &[]))
            .unwrap();
        builder.build()
    }

    let sizing = sizing_for_one_module();
    for (dimension, observed) in [
        (
            CompileLimitDimension::StageScratchBytes,
            sizing.build_scratch_bytes,
        ),
        (
            CompileLimitDimension::CompilerControlledLiveBytes,
            sizing.build_peak_live_bytes,
        ),
    ] {
        let boundary = u32::try_from(observed).unwrap();
        let diagnostics = expect_diagnostics(build_with_limit(dimension, boundary - 1));
        assert!(matches!(
            diagnostics.diagnostics()[0].payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: actual_dimension,
                limit,
                observed: actual_observed,
            } if *actual_dimension == dimension
                && *limit == u64::from(boundary - 1)
                && *actual_observed == observed
        ));

        let unit = build_with_limit(dimension, boundary).unwrap();
        assert_eq!(unit.module_count(), 1);
    }
}

#[test]
fn default_profile_can_freeze_its_maximum_module_count() {
    let limits = CompileLimits::p100_initial_v2();
    let mut builder = CompilationUnitBuilder::new(limits.clone());
    for index in 0..limits.value(CompileLimitDimension::ModuleCount) {
        let namespace = format!("city/{index:03}");
        let document = format!("source/{index:03}");
        builder
            .add_synthetic_module(module_with_document(&namespace, &document, &[]))
            .unwrap();
    }
    let sizing = AdmissionSizing::from_totals(
        builder.totals,
        limits.value(CompileLimitDimension::DiagnosticCount),
    );
    assert!(sizing.build_scratch_bytes <= limits.value(CompileLimitDimension::StageScratchBytes));
    assert!(
        sizing.build_peak_live_bytes
            <= limits.value(CompileLimitDimension::CompilerControlledLiveBytes)
    );

    let unit = builder.build().unwrap();
    assert_eq!(
        u64::try_from(unit.module_count()).unwrap(),
        limits.value(CompileLimitDimension::ModuleCount)
    );
}

#[test]
fn build_limit_diagnostic_uses_canonical_module_context() {
    fn add_modules(builder: &mut CompilationUnitBuilder, namespaces: [&str; 2]) {
        for namespace in namespaces {
            let document = format!("source/{namespace}");
            builder
                .add_synthetic_module(module_with_document(namespace, &document, &[]))
                .unwrap();
        }
    }

    let default_limits = CompileLimits::p100_initial_v2();
    let mut sizing_builder = CompilationUnitBuilder::new(default_limits.clone());
    add_modules(&mut sizing_builder, ["city/z", "city/a"]);
    let sizing = AdmissionSizing::from_totals(
        sizing_builder.totals,
        default_limits.value(CompileLimitDimension::DiagnosticCount),
    );
    let failing_limit = u32::try_from(sizing.build_scratch_bytes).unwrap() - 1;

    for namespaces in [["city/z", "city/a"], ["city/a", "city/z"]] {
        let limits = CompileLimits::p100_initial_v2()
            .with_test_admission_limit(CompileLimitDimension::StageScratchBytes, failing_limit);
        let mut builder = CompilationUnitBuilder::new(limits);
        add_modules(&mut builder, namespaces);
        let diagnostics = expect_diagnostics(builder.build());
        assert_eq!(diagnostics.diagnostics()[0].stable_key(), Some("city/a"));
    }
}

#[test]
fn v2_source_document_count_accepts_boundary_and_rejects_boundary_plus_one_atomically() {
    fn document_inputs(count: usize) -> (Vec<String>, Vec<Vec<u8>>) {
        let keys = (0..count)
            .map(|index| format!("extra/{index:04}"))
            .collect::<Vec<_>>();
        let records = (0..count)
            .map(|index| vec![u8::try_from(index % 251).unwrap()])
            .collect::<Vec<_>>();
        (keys, records)
    }

    let (boundary_keys, boundary_records) = document_inputs(1_565);
    let boundary_inputs = boundary_keys
        .iter()
        .zip(&boundary_records)
        .map(|(key, record)| (key.as_str(), record.as_slice()))
        .collect::<Vec<_>>();
    let mut boundary_builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v2());
    boundary_builder
        .add_test_official_module(TestOfficialModule::from_synthetic_with_documents(
            module_with_document("city/boundary", "primary", &[]),
            &boundary_inputs,
        ))
        .unwrap();
    let boundary_unit = boundary_builder.build().unwrap();
    assert_eq!(boundary_unit.source_document_count(), 1_566);
    let mut compiler = crate::Compiler::new();
    let boundary_output = compiler.compile(boundary_unit).unwrap();
    assert_eq!(
        boundary_output.source_map_input().source_documents().len(),
        1_566
    );

    let (over_keys, over_records) = document_inputs(1_566);
    let over_inputs = over_keys
        .iter()
        .zip(&over_records)
        .map(|(key, record)| (key.as_str(), record.as_slice()))
        .collect::<Vec<_>>();
    let mut over_builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v2());
    let diagnostics = expect_diagnostics(over_builder.add_test_official_module(
        TestOfficialModule::from_synthetic_with_documents(
            module_with_document("city/over", "primary", &[]),
            &over_inputs,
        ),
    ));
    assert!(matches!(
        diagnostics.diagnostics()[0].payload(),
        DiagnosticPayload::CompileLimitExceeded {
            dimension: CompileLimitDimension::SourceDocumentCount,
            limit: 1_566,
            observed: 1_567,
        }
    ));
    over_builder
        .add_synthetic_module(module_with_document("city/over", "primary", &[]))
        .unwrap();
    assert_eq!(over_builder.build().unwrap().source_document_count(), 1);
}

#[test]
fn document_set_digest_and_frozen_document_order_ignore_input_order() {
    let left = TestOfficialModule::from_synthetic_with_documents(
        module_with_document("city/a", "source/m", &[]),
        &[("source/z", b"z"), ("source/a", b"a")],
    );
    let right = TestOfficialModule::from_synthetic_with_documents(
        module_with_document("city/a", "source/m", &[]),
        &[("source/a", b"a"), ("source/z", b"z")],
    );
    assert_eq!(
        left.admitted.descriptor.source_document_set_digest(),
        right.admitted.descriptor.source_document_set_digest()
    );
    assert_eq!(
        left.admitted
            .source_documents
            .iter()
            .map(SourceDocumentDescriptor::source_document_key)
            .collect::<Vec<_>>(),
        ["source/a", "source/m", "source/z"]
    );
}

#[test]
fn source_document_index_and_flat_descriptors_are_in_peak_live_accounting() {
    fn compile_with_extra_documents(documents: &[(&str, &[u8])]) -> u64 {
        let limits = CompileLimits::p100_initial_v2();
        let module = TestOfficialModule::from_synthetic_with_documents(
            module_with_document("city/a", "source/primary", &[]),
            documents,
        );
        let mut builder = CompilationUnitBuilder::new(limits);
        builder.add_test_official_module(module).unwrap();
        crate::Compiler::new()
            .compile(builder.build().unwrap())
            .unwrap()
            .source_map_input()
            .peak_controlled_live_bytes()
    }

    let one_document_peak = compile_with_extra_documents(&[]);
    let extra_documents = [
        ("source/secondary", b"secondary".as_slice()),
        ("source/tertiary", b"tertiary".as_slice()),
    ];
    let three_document_peak = compile_with_extra_documents(&extra_documents);
    let extra_key_bytes = extra_documents.iter().fold(0_u64, |total, (key, _)| {
        total.saturating_add(u64::try_from(key.len()).unwrap_or(u64::MAX))
    });
    let descriptor_bytes = size_bytes::<SourceDocumentDescriptor>(2);
    let index_growth = source_document_index_requested_bytes(3)
        .saturating_sub(source_document_index_requested_bytes(1));
    let expected_growth = extra_key_bytes
        .saturating_add(descriptor_bytes)
        .saturating_add(index_growth)
        .saturating_add(descriptor_bytes);
    assert_eq!(
        three_document_peak.saturating_sub(one_document_peak),
        expected_growth
    );
}

#[test]
fn three_document_module_retains_distinct_entity_relation_and_cold_origins() {
    let limits = CompileLimits::p100_initial_v2();
    let mut synthetic =
        SyntheticModuleBuilder::new(header("city/a", "source/primary"), &limits).unwrap();
    synthetic
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 12.0,
            speed_limit_meters_per_second: 8.0,
            successors: &[LaneEdgeReference::local("edge-b")],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge-b",
            length_meters: 8.0,
            speed_limit_meters_per_second: 6.0,
            successors: &[],
        })
        .unwrap();
    SOURCE_DOCUMENT_DIGEST_CALL_COUNT.with(|count| count.set(0));
    let mut test_module = {
        let secondary_record = b"secondary record".to_vec();
        let tertiary_record = b"tertiary record".to_vec();
        let secondary_origin = String::from("memory://secondary");
        let tertiary_origin = String::from("memory://tertiary");
        TestOfficialModule::from_synthetic_with_document_records(
            synthetic.finish().unwrap(),
            &[
                TestSourceDocument {
                    source_document_key: "source/secondary",
                    source_record: &secondary_record,
                    display_source: Some(&secondary_origin),
                },
                TestSourceDocument {
                    source_document_key: "source/tertiary",
                    source_record: &tertiary_record,
                    display_source: Some(&tertiary_origin),
                },
            ],
        )
    };
    SOURCE_DOCUMENT_DIGEST_CALL_COUNT.with(|count| assert_eq!(count.get(), 3));
    test_module.move_module_declaration_span_to("source/secondary");
    test_module.move_first_lane_edge_span_to("source/secondary");
    test_module.move_first_lane_edge_successor_span_to("source/tertiary");

    let mut unit_builder = CompilationUnitBuilder::new(limits);
    unit_builder.add_test_official_module(test_module).unwrap();
    let mut compiler = crate::Compiler::new();
    let output = compiler.compile(unit_builder.build().unwrap()).unwrap();
    assert_eq!(
        output
            .source_map_input()
            .source_module_sources()
            .map(|source| (
                source.descriptor().authoring_namespace_id(),
                source.primary_source().source_document_key(),
                source.primary_source().start().line(),
                source.primary_source().start().column(),
            ))
            .collect::<Vec<_>>(),
        [("city/a", "source/secondary", 37, 5)]
    );
    assert_eq!(
        output
            .source_map_input()
            .source_documents()
            .map(|document| document.source_document_key())
            .collect::<Vec<_>>(),
        ["source/primary", "source/secondary", "source/tertiary"]
    );
    assert_eq!(
        output
            .source_map_input()
            .lane_edge_sources()
            .filter(|source| {
                source.primary_source().source_document_key() == "source/secondary"
            })
            .count(),
        1
    );
    assert_eq!(
        output
            .source_map_input()
            .lane_edge_successor_sources()
            .next()
            .unwrap()
            .primary_source()
            .source_document_key(),
        "source/tertiary"
    );
    assert_eq!(
        output
            .source_map_input()
            .source_documents()
            .map(|document| (
                document.source_document_key(),
                document.origin().display_source()
            ))
            .collect::<Vec<_>>(),
        [
            ("source/primary", None),
            ("source/secondary", Some("memory://secondary")),
            ("source/tertiary", Some("memory://tertiary")),
        ]
    );
}

#[test]
fn source_map_output_bytes_include_module_declaration_document_ordinal() {
    let limits = CompileLimits::p100_initial_v2();
    let mut module = TestOfficialModule::from_synthetic_with_documents(
        module_with_document("city/a", "source/primary", &[]),
        &[("source/secondary", b"secondary")],
    );
    module.move_module_declaration_span_to("source/secondary");

    let mut builder = CompilationUnitBuilder::new(limits);
    builder.add_test_official_module(module).unwrap();
    let mut unit = builder.build().unwrap();
    let source_map_logical_bytes = unit.modules.iter().fold(0_u64, |total, module| {
        module.source_documents.iter().fold(
            total
                .saturating_add(module.descriptor.source_map_logical_bytes())
                .saturating_add(4 + 8 + 8),
            |document_total, document| {
                document_total.saturating_add(document.source_map_logical_bytes())
            },
        )
    });
    let hir = crate::hir::build_hir(&unit).unwrap();
    let mir = crate::mir::lower_to_mir(&unit, &hir).unwrap();
    let lir_output_bytes = crate::lir::freeze_lir(&unit, &mir)
        .unwrap()
        .lir
        .output_bytes;
    let expected_output_bytes = lir_output_bytes.saturating_add(source_map_logical_bytes);
    let failing_limit = u32::try_from(expected_output_bytes - 1).unwrap();
    unit.limits = CompileLimits::p100_initial_v2().with_test_lir_limits(
        u32::MAX,
        u32::MAX,
        failing_limit,
        u32::MAX,
    );

    let diagnostics = expect_diagnostics(crate::Compiler::new().compile(unit));
    assert!(diagnostics.diagnostics().iter().any(|diagnostic| matches!(
        diagnostic.payload(),
        DiagnosticPayload::CompileLimitExceeded {
            dimension: CompileLimitDimension::OutputBytes,
            limit,
            observed,
        } if *limit == u64::from(failing_limit) && *observed == expected_output_bytes
    )));
}

#[test]
fn source_map_rejects_a_span_bound_to_another_modules_document() {
    let limits = CompileLimits::p100_initial_v2();
    let mut synthetic = SyntheticModuleBuilder::new(header("city/a", "source/a"), &limits).unwrap();
    synthetic
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 12.0,
            speed_limit_meters_per_second: 8.0,
            successors: &[],
        })
        .unwrap();
    let mut test_module =
        TestOfficialModule::from_synthetic_with_documents(synthetic.finish().unwrap(), &[]);
    test_module.move_first_lane_edge_span_to("source/b");

    let mut unit_builder = CompilationUnitBuilder::new(limits);
    unit_builder.add_test_official_module(test_module).unwrap();
    unit_builder
        .add_synthetic_module(module_with_document("city/b", "source/b", &[]))
        .unwrap();
    let diagnostics =
        expect_diagnostics(crate::Compiler::new().compile(unit_builder.build().unwrap()));
    assert!(matches!(
        diagnostics.diagnostics()[0].payload(),
        DiagnosticPayload::SourceDocumentOwnershipMismatch {
            source_document_key,
            expected_authoring_namespace_id,
            actual_authoring_namespace_id: Some(actual),
        } if source_document_key.as_ref() == "source/b"
            && expected_authoring_namespace_id.as_ref() == "city/a"
            && actual.as_ref() == "city/b"
    ));
}

#[test]
fn source_map_rejects_an_unregistered_span_document_without_panicking() {
    let limits = CompileLimits::p100_initial_v2();
    let mut synthetic = SyntheticModuleBuilder::new(header("city/a", "source/a"), &limits).unwrap();
    synthetic
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 12.0,
            speed_limit_meters_per_second: 8.0,
            successors: &[],
        })
        .unwrap();
    let mut test_module =
        TestOfficialModule::from_synthetic_with_documents(synthetic.finish().unwrap(), &[]);
    test_module.move_first_lane_edge_span_to("source/missing");

    let mut unit_builder = CompilationUnitBuilder::new(limits);
    unit_builder.add_test_official_module(test_module).unwrap();
    let diagnostics =
        expect_diagnostics(crate::Compiler::new().compile(unit_builder.build().unwrap()));
    assert!(matches!(
        diagnostics.diagnostics()[0].payload(),
        DiagnosticPayload::SourceDocumentOwnershipMismatch {
            source_document_key,
            expected_authoring_namespace_id,
            actual_authoring_namespace_id: None,
        } if source_document_key.as_ref() == "source/missing"
            && expected_authoring_namespace_id.as_ref() == "city/a"
    ));
}

#[test]
#[ignore = "measurement-only admission benchmark; run explicitly with --release --nocapture"]
fn benchmark_common_admission_only_reports_median_mad_and_memory() {
    use std::hint::black_box;
    use std::time::Instant;

    const MODULE_COUNT: usize = 100;
    const SAMPLE_COUNT: usize = 7;

    fn fixture() -> Vec<TestOfficialModule> {
        (0..MODULE_COUNT)
            .map(|index| {
                let namespace = format!("bench/module-{index:03}");
                let primary = format!("bench/{index:03}/primary");
                let secondary = format!("bench/{index:03}/secondary");
                let tertiary = format!("bench/{index:03}/tertiary");
                let secondary_origin = format!("memory://bench/{index:03}/secondary");
                let tertiary_origin = format!("memory://bench/{index:03}/tertiary");
                TestOfficialModule::from_synthetic_with_document_records(
                    module_with_document(&namespace, &primary, &[]),
                    &[
                        TestSourceDocument {
                            source_document_key: &secondary,
                            source_record: b"secondary",
                            display_source: Some(&secondary_origin),
                        },
                        TestSourceDocument {
                            source_document_key: &tertiary,
                            source_record: b"tertiary",
                            display_source: Some(&tertiary_origin),
                        },
                    ],
                )
            })
            .collect()
    }

    fn run_once(modules: Vec<TestOfficialModule>) -> (u128, AdmissionSizing, u64) {
        let origin_bytes = modules
            .iter()
            .flat_map(|module| module.admitted.source_documents.iter())
            .filter_map(|document| document.origin.display_source.as_ref())
            .fold(0_u64, |total, origin| {
                total.saturating_add(u64::try_from(origin.len()).unwrap_or(u64::MAX))
            });
        let start = Instant::now();
        let limits = CompileLimits::p100_initial_v2();
        let diagnostic_limit = limits.value(CompileLimitDimension::DiagnosticCount);
        let mut builder = CompilationUnitBuilder::new(limits);
        for module in modules {
            builder.add_test_official_module(module).unwrap();
        }
        let totals = builder.totals;
        let unit = builder.build().unwrap();
        let elapsed = start.elapsed().as_nanos();
        let sizing = AdmissionSizing::from_totals(totals, diagnostic_limit);
        let result_live_bytes = black_box(unit.controlled_live_bytes);
        assert_eq!(result_live_bytes, sizing.result_live_bytes);
        black_box(unit);
        (elapsed, sizing, origin_bytes)
    }

    let _warmup = run_once(fixture());
    let mut samples = Vec::with_capacity(SAMPLE_COUNT);
    let mut sizing = AdmissionSizing {
        builder_live_bytes: 0,
        result_live_bytes: 0,
        build_scratch_bytes: 0,
        build_peak_live_bytes: 0,
    };
    let mut origin_bytes = 0_u64;
    for _ in 0..SAMPLE_COUNT {
        let (elapsed, measured_sizing, measured_origin_bytes) = run_once(fixture());
        samples.push(elapsed);
        sizing = measured_sizing;
        origin_bytes = measured_origin_bytes;
    }
    samples.sort_unstable();
    let median_ns = samples[SAMPLE_COUNT / 2];
    let mut deviations = samples
        .iter()
        .map(|sample| sample.abs_diff(median_ns))
        .collect::<Vec<_>>();
    deviations.sort_unstable();
    let mad_ns = deviations[SAMPLE_COUNT / 2];
    println!(
        "admission-only modules={MODULE_COUNT} documents={} warmups=1 samples={SAMPLE_COUNT} median_ns={median_ns} mad_ns={mad_ns} origin_bytes={origin_bytes} builder_live_bytes={} result_live_bytes={} build_scratch_bytes={} admission_peak_live_bytes={}",
        MODULE_COUNT * 3,
        sizing.builder_live_bytes,
        sizing.result_live_bytes,
        sizing.build_scratch_bytes,
        sizing.build_peak_live_bytes,
    );
}
