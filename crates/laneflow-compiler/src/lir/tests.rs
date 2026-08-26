use super::*;
use crate::hir::build_hir;
use crate::identity::{IdentityFieldInput, encode_canonical_identity};
use crate::mir::lower_to_mir;
use crate::{
    CompilationUnitBuilder, CompileLimits, DiagnosticPayload, LaneEdgeInput, LaneEdgeReference,
    SourceModuleHeader, SourceModuleHeaderInput, SyntheticModule, SyntheticModuleBuilder,
};

fn module(
    namespace: &str,
    source_document_key: &str,
    imports: &[&str],
    edges: &[(&str, f64, &[LaneEdgeReference<'_>])],
) -> SyntheticModule {
    let limits = CompileLimits::p100_initial_v1();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: namespace,
            source_document_key,
            generator_build_id: "git:0123456789abcdef",
            parameters_and_inputs_digest: [0x11; 32],
            frontend_options_digest: [0x22; 32],
            random_seed: Some(42),
            provenance: "repository:laneflow",
        },
        &limits,
    )
    .unwrap();
    let mut builder = SyntheticModuleBuilder::new(header, &limits).unwrap();
    for import in imports {
        builder.add_import(import).unwrap();
    }
    for (key, length_meters, successors) in edges {
        builder
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: key,
                length_meters: *length_meters,
                speed_limit_meters_per_second: 13.75,
                successors,
            })
            .unwrap();
    }
    builder.finish().unwrap()
}

fn unit(modules: impl IntoIterator<Item = SyntheticModule>) -> CompilationUnit {
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    for module in modules {
        builder.add_synthetic_module(module).unwrap();
    }
    builder.build().unwrap()
}

fn lir(unit: &CompilationUnit) -> LirUnit {
    let hir = build_hir(unit).unwrap();
    let mir = lower_to_mir(unit, &hir).unwrap();
    freeze_lir(unit, &mir).unwrap().lir
}

fn identity_values(lir: &LirUnit, edge: &LirLaneEdge) -> Vec<(FieldTag, Vec<u8>)> {
    lir.identity_fields[edge.identity_fields.as_usize_range()]
        .iter()
        .map(|field| {
            (
                field.tag,
                lir.identity_field_bytes[field.value_bytes.as_usize_range()].to_vec(),
            )
        })
        .collect()
}

#[test]
fn lir_sorts_by_complete_identity_bytes_and_remaps_connections() {
    let app_successors = [LaneEdgeReference::imported("z", "edge-z")];
    let unit = unit([
        module("a", "app", &["z"], &[("edge-a", 10.0, &app_successors)]),
        module("z", "base", &[], &[("edge-z", 20.0, &[])]),
    ]);
    let lir = lir(&unit);

    assert_eq!(lir.lane_edges.len(), 2);
    assert_eq!(lir.lane_edge_successors.len(), 1);
    assert_eq!(lir.lir_record_count, 3);
    assert_eq!(lir.output_bytes, 180);
    assert!(lir.controlled_live_bytes > 0);
    assert_eq!(lir.lane_edges[0].ordinal.raw(), 0);
    assert_eq!(lir.lane_edges[1].ordinal.raw(), 1);
    assert_eq!(
        identity_values(&lir, &lir.lane_edges[0]),
        [
            (FieldTag::AuthoringNamespaceId, b"a".to_vec()),
            (FieldTag::LaneEdgeKey, b"edge-a".to_vec()),
        ]
    );
    assert_eq!(
        identity_values(&lir, &lir.lane_edges[1]),
        [
            (FieldTag::AuthoringNamespaceId, b"z".to_vec()),
            (FieldTag::LaneEdgeKey, b"edge-z".to_vec()),
        ]
    );
    assert_eq!(
        lir.lane_edge_successors[lir.lane_edges[0].successors.as_usize_range()][0].raw(),
        1
    );
}

#[test]
fn identity_order_uses_little_endian_length_prefix_before_text_bytes() {
    let unit = unit([
        module("aa", "aa", &[], &[("edge", 10.0, &[])]),
        module("z", "z", &[], &[("edge", 10.0, &[])]),
    ]);
    let lir = lir(&unit);

    // 普通文本顺序是 "aa" < "z"，但 Identity v1 在字段值前编码 u32_le 长度；
    // 完整前像的第一个差异字节因此是 1 < 2。
    assert_eq!(
        identity_values(&lir, &lir.lane_edges[0])[0].1,
        b"z".to_vec()
    );
    assert_eq!(
        identity_values(&lir, &lir.lane_edges[1])[0].1,
        b"aa".to_vec()
    );
}

#[test]
fn allocation_free_sort_key_matches_the_identity_v1_encoder() {
    let namespaces = [b"a".as_slice(), b"aa", b"z", b"city/a"];
    let keys = [b"e".as_slice(), b"edge", b"edge-00", b"edge-longer"];

    for left_namespace in namespaces {
        for left_key in keys {
            let left_fields = [
                IdentityFieldInput::new(FieldTag::AuthoringNamespaceId, left_namespace),
                IdentityFieldInput::new(FieldTag::LaneEdgeKey, left_key),
            ];
            let left = encode_canonical_identity(EntityKind::LaneEdge, &left_fields, 53).unwrap();
            for right_namespace in namespaces {
                for right_key in keys {
                    let right_fields = [
                        IdentityFieldInput::new(FieldTag::AuthoringNamespaceId, right_namespace),
                        IdentityFieldInput::new(FieldTag::LaneEdgeKey, right_key),
                    ];
                    let right =
                        encode_canonical_identity(EntityKind::LaneEdge, &right_fields, 53).unwrap();

                    assert_eq!(
                        compare_lane_edge_identity_fields(
                            left_namespace,
                            left_key,
                            right_namespace,
                            right_key,
                        ),
                        left.canonical_bytes().cmp(right.canonical_bytes())
                    );
                }
            }
        }
    }
}

#[test]
fn semantic_digest_is_invariant_to_declaration_order_and_source_spans() {
    let successors = [
        LaneEdgeReference::local("edge-c"),
        LaneEdgeReference::local("edge-b"),
    ];
    let left = unit([module(
        "city/a",
        "left.document",
        &[],
        &[
            ("edge-a", 10.0, &successors),
            ("edge-b", 20.0, &[]),
            ("edge-c", 30.0, &[]),
        ],
    )]);
    let right = unit([module(
        "city/a",
        "right.document",
        &[],
        &[
            ("edge-c", 30.0, &[]),
            ("edge-a", 10.0, &successors),
            ("edge-b", 20.0, &[]),
        ],
    )]);

    assert_eq!(lir(&left).semantic_digest, lir(&right).semantic_digest);
}

#[test]
fn semantic_digest_changes_with_static_semantics() {
    let left = unit([module(
        "city/a",
        "same.document",
        &[],
        &[("edge-a", 10.0, &[])],
    )]);
    let right = unit([module(
        "city/a",
        "same.document",
        &[],
        &[("edge-a", 11.0, &[])],
    )]);

    assert_ne!(lir(&left).semantic_digest, lir(&right).semantic_digest);
}

#[test]
fn lir_checks_record_scratch_output_and_live_limits_before_allocation() {
    let successors = [LaneEdgeReference::local("edge-a")];
    let mut unit = unit([module(
        "city/a",
        "city/a",
        &[],
        &[("edge-a", 10.0, &successors)],
    )]);
    let hir = build_hir(&unit).unwrap();
    let mir = lower_to_mir(&unit, &hir).unwrap();

    for (limits, expected_dimension) in [
        (
            CompileLimits::p100_initial_v1().with_test_lir_limits(1, u32::MAX, u32::MAX, u32::MAX),
            CompileLimitDimension::LirRecordCount,
        ),
        (
            CompileLimits::p100_initial_v1().with_test_lir_limits(u32::MAX, 0, u32::MAX, u32::MAX),
            CompileLimitDimension::StageScratchBytes,
        ),
        (
            CompileLimits::p100_initial_v1().with_test_lir_limits(u32::MAX, u32::MAX, 0, u32::MAX),
            CompileLimitDimension::OutputBytes,
        ),
        (
            CompileLimits::p100_initial_v1().with_test_lir_limits(
                u32::MAX,
                u32::MAX,
                u32::MAX,
                u32::try_from(unit.controlled_live_bytes + mir.controlled_live_bytes).unwrap(),
            ),
            CompileLimitDimension::CompilerControlledLiveBytes,
        ),
    ] {
        unit.limits = limits;
        // 资源限制来自同一不可变配置档；测试只替换限制快照，不改变 MIR 语义。
        let failure = match freeze_lir(&unit, &mir) {
            Ok(_) => panic!("LIR resource limit must fail closed"),
            Err(diagnostics) => diagnostics,
        };
        assert!(failure.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic.payload(),
            DiagnosticPayload::CompileLimitExceeded { dimension, .. }
                if *dimension == expected_dimension
        )));
    }
}
