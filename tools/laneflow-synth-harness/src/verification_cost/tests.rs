use std::num::NonZeroU32;

use laneflow_synth_generator::{Identity, Provenance, generate};

use super::*;

#[cfg(feature = "verification-cost-counts")]
#[global_allocator]
static ALLOCATOR: &stats_alloc::StatsAlloc<std::alloc::System> = &stats_alloc::INSTRUMENTED_SYSTEM;

fn execution() -> ExecutionConfig {
    ExecutionConfig::new(NonZeroU32::MIN)
}

fn fixture(case: Case) -> (Bundle, Plan) {
    let bundle = generate(
        Identity::new(10_000, case, 16).unwrap(),
        Provenance {
            source_commit: "a".repeat(40),
            generator_commit: "b".repeat(40),
        },
    )
    .unwrap();
    let mut plan = Plan::read(&bundle).unwrap();
    // 仅内部接线回归压缩时间；公开入口仍先验证冻结字节和完整 Plan。
    let old_warm = plan.warm_up_ticks;
    plan.warm_up_ticks = 1;
    plan.observation_ticks = 2;
    for batch in &mut plan.scheduled {
        let boundary = batch["boundary"].as_u64().unwrap();
        if boundary > 0 {
            batch["boundary"] = json!(boundary + 1 - old_warm);
        }
    }
    (bundle, plan)
}

fn same_files(left: &Path, right: &Path, names: &[&str]) {
    for name in names {
        assert_eq!(
            std::fs::read(left.join(name)).unwrap(),
            std::fs::read(right.join(name)).unwrap(),
            "{name} 不得因插桩改变"
        );
    }
}

#[test]
fn burst_timing_preserves_semantics_and_identifies_each_phase_boundary() {
    let (bundle, plan) = fixture(Case::W4);
    let root = tempfile::tempdir().unwrap();
    let plain = root.path().join("plain");
    let timed = root.path().join("timed");
    std::fs::create_dir(&plain).unwrap();
    std::fs::create_dir(&timed).unwrap();
    let implementation = "c".repeat(40);
    let expected = crate::run_plan(
        &bundle,
        &plan,
        &implementation,
        execution(),
        &plain,
        &mut Noop,
    )
    .unwrap();
    let mut probe = Timing::new(&implementation, execution());
    let actual = crate::run_plan(
        &bundle,
        &plan,
        &implementation,
        execution(),
        &timed,
        &mut probe,
    )
    .unwrap();
    assert_eq!(actual, expected);
    same_files(
        &plain,
        &timed,
        &["ticks.bin", "events.bin", "commands.bin", "batches.json"],
    );
    let cost = probe.report(true).unwrap();
    assert_eq!(cost["completed_boundaries"], 4);
    assert_eq!(cost["observation_ns"]["initial"]["samples"], 1);
    assert_eq!(cost["observation_ns"]["warm_up"]["samples"], 1);
    let observation = &cost["observation_ns"]["observation"];
    assert_eq!(observation["samples"], 2);
    assert_eq!(observation["raw_ns"].as_array().unwrap().len(), 2);
    for percentile in ["p50_ns", "p95_ns", "p99_ns"] {
        assert!(observation[percentile].is_null());
    }
    let rows = cost["boundary_samples"].as_array().unwrap();
    assert_eq!(rows[1]["tick"], 1);
    assert_eq!(rows[1]["observation_scope"], "warm_up");
    assert_eq!(rows[1]["command_boundary"], 1);
    assert_eq!(rows[1]["runtime_step_tick"], 2);
    assert_eq!(rows[2]["observation_scope"], "observation");
    assert_eq!(rows[2]["command_boundary"], 2);
    assert_eq!(rows[2]["runtime_step_tick"], 3);
    assert!(rows[3]["command_boundary"].is_null());
    assert!(rows[3]["runtime_step_tick"].is_null());

    #[cfg(feature = "verification-cost-counts")]
    {
        let counted = root.path().join("counted");
        std::fs::create_dir(&counted).unwrap();
        let mut probe = counts::Counts::new(&implementation, execution());
        assert_eq!(
            crate::run_plan(
                &bundle,
                &plan,
                &implementation,
                execution(),
                &counted,
                &mut probe
            )
            .unwrap(),
            expected
        );
        same_files(
            &plain,
            &counted,
            &["ticks.bin", "events.bin", "commands.bin", "batches.json"],
        );
        let cost = probe.report(true).unwrap();
        let work = &cost["workload_counts"];
        // 真实 W4：初始 7,500 Active；B−1 完成 100 辆；B0 替换并离位后
        // 7,600 Active。K 必须来自实际 push，不能以总人口 10,000 代替。
        assert_eq!(work["initial"]["body_fragments"], 7_500);
        assert_eq!(work["warm_up"]["body_fragments"], 7_400);
        assert_eq!(work["observation"]["body_fragments"], 15_200);
        assert_eq!(
            work["observation"]["maximum_body_fragments_per_boundary"],
            7_600
        );
        assert_eq!(
            work["warm_up"]["scans"]["body_order"]["visited_rows"],
            7_500
        );
        assert_eq!(
            work["observation"]["scans"]["vehicle_state"]["visited_rows"],
            20_000
        );
        assert_eq!(
            work["observation"]["scans"]["transition_events"]["visited_rows"],
            200
        );
        assert!(cost["whole_run_wall_ns"].is_null());
        assert!(cost["observation_ns"].is_null());
        // 并行测试进程的全局分配差值会受其他测试干扰，不据此断言精确预算。
    }
}

#[test]
fn ordinary_timing_preserves_incomplete_short_fixture_and_step_alignment() {
    let (bundle, plan) = fixture(Case::W1);
    let root = tempfile::tempdir().unwrap();
    let plain = root.path().join("plain");
    let timed = root.path().join("timed");
    std::fs::create_dir(&plain).unwrap();
    std::fs::create_dir(&timed).unwrap();
    let implementation = "c".repeat(40);
    let expected = crate::run_plan(
        &bundle,
        &plan,
        &implementation,
        execution(),
        &plain,
        &mut Noop,
    )
    .unwrap_err();
    let mut probe = Timing::new(&implementation, execution());
    let actual = crate::run_plan(
        &bundle,
        &plan,
        &implementation,
        execution(),
        &timed,
        &mut probe,
    )
    .unwrap_err();
    assert_eq!(actual.to_string(), expected.to_string());
    same_files(&plain, &timed, &["ticks.bin", "events.bin"]);
    let cost = probe.report(false).unwrap();
    assert_eq!(cost["complete"], false);
    assert_eq!(cost["completed_boundaries"], 4);
    let rows = cost["boundary_samples"].as_array().unwrap();
    assert!(rows[0]["runtime_step_tick"].is_null());
    assert_eq!(rows[1]["runtime_step_tick"], 1);
    assert_eq!(rows[2]["runtime_step_tick"], 2);
    assert_eq!(rows[3]["runtime_step_tick"], 3);
    assert!(rows.iter().all(|row| row["command_boundary"].is_null()));

    #[cfg(feature = "verification-cost-counts")]
    {
        let counted = root.path().join("counted");
        std::fs::create_dir(&counted).unwrap();
        let mut probe = counts::Counts::new(&implementation, execution());
        let error = crate::run_plan(
            &bundle,
            &plan,
            &implementation,
            execution(),
            &counted,
            &mut probe,
        )
        .unwrap_err();
        assert_eq!(error.to_string(), expected.to_string());
        same_files(&plain, &counted, &["ticks.bin", "events.bin"]);
        let cost = probe.report(false).unwrap();
        let work = &cost["workload_counts"];
        assert_eq!(work["initial"]["body_fragments"], 7_500);
        assert_eq!(work["observation"]["body_fragments"], 15_000);
        assert_eq!(
            work["observation"]["scans"]["vehicle_state"]["visited_rows"],
            20_000
        );
        // 普通 W 将区间检查融合在车辆遍历中，不能虚报第二次 body 扫描。
        assert_eq!(work["observation"]["scans"]["body_order"]["invocations"], 0);
    }
}

#[test]
#[cfg(not(feature = "allocation"))]
fn public_failure_stays_incomplete_and_existing_output_is_preserved() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("missing");
    let output = root.path().join("output");
    let mode = if cfg!(feature = "verification-cost-counts") {
        Mode::Counts
    } else {
        Mode::Timing
    };
    let implementation = "c".repeat(40);
    assert!(run(&input, &implementation, execution(), mode, &output).is_err());
    let original = std::fs::read(output.join(FILE)).unwrap();
    let cost: Value = serde_json::from_slice(&original).unwrap();
    assert_eq!(cost["complete"], false);
    assert!(cost["error"].as_str().is_some());
    let semantic: Value =
        serde_json::from_slice(&std::fs::read(output.join("report.json")).unwrap()).unwrap();
    assert_eq!(semantic["complete"], false);
    assert!(run(&input, &implementation, execution(), mode, &output).is_err());
    assert_eq!(std::fs::read(output.join(FILE)).unwrap(), original);
}

#[test]
fn mismatched_mode_rejects_before_output_creation() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("output");
    let mode = if cfg!(feature = "verification-cost-counts") {
        Mode::Timing
    } else {
        Mode::Counts
    };
    assert!(run(root.path(), &"c".repeat(40), execution(), mode, &output).is_err());
    assert!(!output.exists());
}
