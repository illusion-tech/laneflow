use std::cell::Cell;

use super::*;
use crate::ledger::LedgerStage;
use crate::portable_emitter::budget::test_accounting;

/// 覆盖全部存储段的小对象：字符串、字节、摘要、序号向量与嵌套记录。
fn emit_sample(t: &TreeBuilder, extra_rows: usize) -> [SectionRef; 2] {
    let first = t.section(
        1,
        [t.table(
            1,
            [t.row([
                field(1, Value::U16(1)),
                field(2, t.utf8("laneflow")),
                field(3, t.sha256([7; 32])),
            ])],
        )],
    );
    let second = t.section(
        2,
        [
            t.table(
                1,
                (0..2 + extra_rows).map(|index| {
                    t.row([
                        Some(field(1, Value::U32(u32::try_from(index).unwrap()))),
                        Some(field(2, t.bytes(&[1, 2, 3]))),
                        (index % 2 == 0).then(|| field(3, t.ordinals([4, 5, 6]))),
                        Some(field(
                            4,
                            t.records((0..index).map(|n| {
                                t.record([field(1, Value::U32(u32::try_from(n).unwrap()))])
                            })),
                        )),
                    ])
                }),
            ),
            t.table(2, []),
        ],
    );
    [first, second]
}

fn sample_shape() -> TreeShape {
    let counter = TreeBuilder::counting(PassLabel::new(LedgerStage::SingleEmission, "count"));
    emit_sample(&counter, 0);
    counter.shape().unwrap()
}

#[test]
fn counting_mode_plans_exactly_what_building_mode_grants() {
    let shape = sample_shape();
    assert_eq!(
        shape,
        TreeShape {
            sections: 2,
            tables: 3,
            rows: 3,
            fields: 3 + 4 + 3,
            records: 1,
            record_fields: 1,
            bytes: 8 + 32 + 3 + 3,
            ordinals: 3,
        }
    );
    let accounting = test_accounting(u64::MAX);
    let tree = build_tree(
        accounting,
        "sample-tree",
        PortableObjectKind::SemanticDiff,
        |t| Ok::<_, PortableEmissionError>(emit_sample(t, 0)),
    )
    .unwrap();
    assert_eq!(Some(tree.granted_bytes()), shape.plan().bytes());
    assert_eq!(accounting.ledger.live(), tree.granted_bytes());
    assert_eq!(accounting.ledger.reserved(), 0);
    drop(tree);
    assert_eq!(accounting.ledger.live(), 0);
}

#[test]
fn tree_plan_above_the_budget_is_rejected_before_any_tree_storage() {
    let plan = sample_shape().plan().bytes().unwrap();
    let accounting = test_accounting(plan - 1);
    let error = build_tree(
        accounting,
        "sample-tree",
        PortableObjectKind::SemanticDiff,
        |t| Ok::<_, PortableEmissionError>(emit_sample(t, 0)),
    )
    .err()
    .unwrap();
    assert_eq!(
        error,
        PortableEmissionError::CompileLimitExceeded {
            dimension: crate::CompileLimitDimension::CompilerControlledLiveBytes,
            actual: plan,
            limit: plan - 1,
        }
    );
    assert_eq!(accounting.ledger.live(), 0);
    assert_eq!(accounting.ledger.reserved(), 0);

    let exact = test_accounting(plan);
    build_tree(
        exact,
        "sample-tree",
        PortableObjectKind::SemanticDiff,
        |t| Ok::<_, PortableEmissionError>(emit_sample(t, 0)),
    )
    .unwrap();
}

#[test]
fn building_mode_writing_more_or_less_than_counted_is_an_accounting_defect() {
    for extra in [1, 0] {
        let runs = Cell::new(0);
        let accounting = test_accounting(u64::MAX);
        let error = build_tree(
            accounting,
            "sample-tree",
            PortableObjectKind::SemanticDiff,
            |t| {
                // 计数一遍按 1 行多写或少写，构建一遍按另一种写入量。
                let run = runs.replace(runs.get() + 1);
                let rows = if run == 0 { 1 - extra } else { extra };
                Ok::<_, PortableEmissionError>(emit_sample(t, rows))
            },
        )
        .err()
        .unwrap();
        assert_eq!(
            error,
            PortableEmissionError::InternalAccountingDefect {
                point: "sample-tree"
            },
            "extra={extra}"
        );
        assert_eq!(accounting.ledger.live(), 0);
        assert_eq!(accounting.ledger.reserved(), 0);
    }
}

#[test]
fn children_must_be_written_contiguously() {
    let accounting = test_accounting(u64::MAX);
    let error = build_tree(
        accounting,
        "sample-tree",
        PortableObjectKind::SemanticDiff,
        |t| {
            let first = t.row([field(1, Value::U8(1))]);
            let second = t.row([field(1, Value::U8(2))]);
            Ok::<_, PortableEmissionError>([t.section(1, [t.table(1, [second, first])])])
        },
    )
    .err()
    .unwrap();
    assert_eq!(error, PortableEmissionError::InternalBindingMismatch);

    let nested = build_tree(
        accounting,
        "sample-tree",
        PortableObjectKind::SemanticDiff,
        |t| {
            let inner = t.records([t.record([field(1, Value::U8(1))])]);
            let outer = t.records([t.record([field(1, inner)])]);
            Ok::<_, PortableEmissionError>([t.section(1, [t.table(1, [t.row([field(1, outer)])])])])
        },
    )
    .err()
    .unwrap();
    assert_eq!(nested, PortableEmissionError::InternalBindingMismatch);
    assert_eq!(accounting.ledger.live(), 0);
}

#[test]
fn owned_objects_round_trip_through_the_tree() {
    let accounting = test_accounting(u64::MAX);
    let tree = build_tree(
        accounting,
        "sample-tree",
        PortableObjectKind::SemanticDiff,
        |t| Ok::<_, PortableEmissionError>(emit_sample(t, 3)),
    )
    .unwrap();
    let owned = tree.to_owned_object();
    let copied = build_tree(
        accounting,
        "copied-tree",
        PortableObjectKind::SemanticDiff,
        |t| Ok::<_, PortableEmissionError>(t.copy_owned(&owned)),
    )
    .unwrap();
    assert_eq!(copied.to_owned_object(), owned);
    assert_eq!(copied.granted_bytes(), tree.granted_bytes());
}

#[test]
fn emission_report_records_transfer_to_host_and_released_working_sets() {
    let candidate =
        crate::compiler::portable_fixture_tests::full_spatial_portable_fixture_candidate();
    let report = candidate.resource_report();
    let objects = [
        candidate.canonical_artifact(),
        candidate.source_map(),
        candidate.semantic_diff(),
    ];
    // 每份内存候选转交原位编码的 Arc<[u8]> 与 object key，另有一份编译器构建标识副本。
    let transferred = objects
        .iter()
        .map(|object| {
            let length = usize::try_from(object.byte_length().get()).unwrap();
            arc_bytes_layout(length).unwrap().size() as u64 + super::super::OBJECT_KEY_BYTES as u64
        })
        .sum::<u64>()
        + candidate.compiler_build_id().len() as u64;
    assert_eq!(report.transferred_bytes(), transferred);
    assert!(report.released_bytes() > 0);
    assert!(report.single_emission_peak_bytes() > 0);
    assert!(report.diff_emission_peak_bytes() > 0);
    assert!(report.peak_live_bytes() >= report.single_emission_peak_bytes());
    assert!(report.peak_live_bytes() >= report.diff_emission_peak_bytes());
}
