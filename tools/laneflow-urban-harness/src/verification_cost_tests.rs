//! 计数测试经独立 CLI 子进程运行，父进程的并行测试分配不进入计数区间。

use std::{path::Path, process::Command};

use laneflow_urban_generator::{Scale, UrbanConfig, generate};
#[cfg(not(any(
    feature = "allocation",
    feature = "execution-evidence",
    feature = "motion-kernel-evidence",
    feature = "pose-profiling"
)))]
use laneflow_urban_harness::compare_runs;
use laneflow_urban_harness::{Artifacts, ResolvedPlan, Window};
#[cfg(not(any(
    feature = "allocation",
    feature = "execution-evidence",
    feature = "motion-kernel-evidence",
    feature = "pose-profiling"
)))]
use serde_json::Value;

fn fixture(temp: &tempfile::TempDir) -> (std::path::PathBuf, std::path::PathBuf, ResolvedPlan) {
    let source = temp.path().join("source");
    let config =
        UrbanConfig::parse(include_str!("../../../examples/config/cn-urban.toml")).unwrap();
    generate(&config, Scale::Fixture, &source, None).unwrap();
    let artifacts = Artifacts::load(&source).unwrap();
    let plan = ResolvedPlan::mixed(&artifacts, Window::probe_after(2, 6).unwrap()).unwrap();
    let path = temp.path().join("plan.toml");
    plan.write(&path).unwrap();
    (source, path, plan)
}

fn cost(
    source: &Path,
    plan: &Path,
    output: &Path,
    mode: &str,
    workers: u32,
) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_laneflow-urban-harness"))
        .arg("verification-cost")
        .arg(source)
        .arg(plan)
        .arg(output)
        .args(["--mode", mode, "--workers", &workers.to_string()])
        .env_remove("LANEFLOW_ENVIRONMENT_INPUT")
        .output()
        .unwrap()
}

#[cfg(not(any(
    feature = "allocation",
    feature = "execution-evidence",
    feature = "motion-kernel-evidence",
    feature = "pose-profiling"
)))]
fn read(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

#[cfg(not(any(
    feature = "allocation",
    feature = "execution-evidence",
    feature = "motion-kernel-evidence",
    feature = "pose-profiling"
)))]
#[test]
fn complete_cost_probe_keeps_original_replay_and_separates_builds() {
    use std::fs;

    let temp = tempfile::tempdir().unwrap();
    let (source, plan_path, plan) = fixture(&temp);
    let (mode, wrong_mode) = if cfg!(feature = "verification-cost-counts") {
        ("counts", "timing")
    } else {
        ("timing", "counts")
    };
    let one = temp.path().join("cost-one");
    let four = temp.path().join("cost-four");
    for (path, workers) in [(&one, 1), (&four, 4)] {
        let result = cost(&source, &plan_path, path, mode, workers);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let regular = temp.path().join("regular");
    let result = Command::new(env!("CARGO_BIN_EXE_laneflow-urban-harness"))
        .arg("run")
        .arg(&source)
        .arg(&plan_path)
        .arg(&regular)
        .arg("--diagnostics")
        .env_remove("LANEFLOW_ENVIRONMENT_INPUT")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    // 诊断对照 D 分支：普通 run 与成本入口都携带专用诊断结果版本，正式
    // compare 在身份门拒绝（probe-match 接受语义属 main 正式版本）；
    // 计量入口与普通 run 的语义等值由下方逐字节比较证明。
    for pair in [&regular, &four] {
        assert!(
            compare_runs(&one, pair)
                .unwrap_err()
                .to_string()
                .contains("cannot accept")
        );
    }
    for name in ["ticks.jsonl", "commands.jsonl", "events.jsonl"] {
        assert_eq!(
            fs::read(one.join(name)).unwrap(),
            fs::read(regular.join(name)).unwrap()
        );
    }
    let receipt = read(&one.join("verification-cost.json"));
    assert_eq!(receipt["format"], "urban-verification-cost-1");
    assert_eq!(receipt["complete"], true);
    assert_eq!(receipt["phase"], "finished");
    assert_eq!(receipt["purpose"], "verification-cost");
    assert_eq!(receipt["window"]["purpose"], "probe");
    assert_eq!(receipt["semantic_run"]["validate_case"]["executed"], false);
    assert_eq!(receipt["samples"].as_array().unwrap().len(), 8);
    assert!(receipt.get("performance_accepted").is_none());
    if mode == "counts" {
        assert!(receipt["whole_run_wall_ns"].is_null());
        assert!(receipt["observation_ns"].is_null());
        assert_eq!(receipt["existing_timing_artifacts_usable"], false);
        let counts = &receipt["workload_counts"]["all"];
        assert_eq!(counts["pre_step_scans"], 8);
        assert_eq!(counts["state_scans"], 8);
        assert_eq!(counts["state_slot_visits"], u64::from(plan.individuals) * 8);
        // 同一块账本先供 pre_step 使用，再清零供 state 使用；前后两段
        // 查询不可丢失或跨阶段重复累计。step 本身不创建/删除 live 槽位。
        let pre_lookups = counts["pre_step_vehicle_lookups"].as_u64().unwrap();
        assert!(pre_lookups > 0);
        assert_eq!(counts["state_vehicle_lookups"], pre_lookups * 2);
        assert_eq!(counts["pre_step_blocks"], counts["state_blocks"]);
        // 诊断对照 D 的计数口径：不做非空校验（0 次扫描 0 行）；相邻对
        // 枚举每拍一次、访问 K-1 对（8 拍合计 fragments-8 行）。
        assert_eq!(counts["body_nonempty_scans"], 0);
        assert_eq!(counts["body_nonempty_scan_rows"], 0);
        assert_eq!(counts["body_overlap_scans"], 8);
        assert!(counts["body_fragments"].as_u64().unwrap() > 8);
        assert_eq!(
            counts["body_overlap_scan_rows"],
            counts["body_fragments"].as_u64().unwrap() - 8
        );
        assert_eq!(counts["body_fragments"], counts["body_sort_input_rows"]);
        let other = read(&four.join("verification-cost.json"));
        assert_eq!(receipt["workload_counts"], other["workload_counts"]);
        for phase in ["pre_step", "post_step"] {
            assert!(
                receipt["allocation"]["all"][phase]["allocations"]
                    .as_u64()
                    .unwrap()
                    > 0
            );
        }
        let capacity = &receipt["scratch_high_water"];
        assert!(capacity["initial"].is_object());
        assert!(capacity["after_first_tick"].is_object());
        let storage = &capacity["instrumentation_storage"];
        assert!(storage["ledger_payload_bytes_per_block"].as_u64().unwrap() > 0);
        assert!(
            storage["maximum_reserved_ledger_payload_bytes"]
                .as_u64()
                .unwrap()
                > 0
        );
        assert!(
            capacity["values"]["block_body_capacity_sum"]
                .as_u64()
                .unwrap()
                > 0
        );
        assert!(capacity["sampled_high_water_increases"]["later_ticks"].is_object());
    } else {
        assert!(receipt["whole_run_wall_ns"].as_u64().unwrap() > 0);
        assert!(receipt["allocation"].is_null());
        assert!(receipt["workload_counts"].is_null());
        assert!(receipt["scratch_high_water"].is_null());
        let samples = receipt["samples"].as_array().unwrap();
        let total: u64 = samples
            .iter()
            .map(|row| row["observation_ns"].as_u64().unwrap())
            .sum();
        assert_eq!(receipt["observation_ns"]["all"]["total_ns"], total);
        let diagnostic = read(&one.join("diagnostics.json"));
        assert_eq!(
            diagnostic["window_observation_samples_ns"],
            Value::from(
                samples[2..]
                    .iter()
                    .map(|row| row["observation_ns"].as_u64().unwrap())
                    .collect::<Vec<_>>()
            )
        );
    }
    let wrong = temp.path().join("wrong-build");
    let result = cost(&source, &plan_path, &wrong, wrong_mode, 1);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("需要"));
    assert!(!wrong.exists());
    let existing = cost(&source, &plan_path, &one, mode, 1);
    assert!(!existing.status.success());
    assert_eq!(read(&one.join("verification-cost.json")), receipt);

    // 即便文件可解码，也不得把产品 performance 计划当验证成本窗口消费。
    let mut performance = plan;
    performance.window.purpose = "performance".into();
    let performance_path = temp.path().join("performance.toml");
    performance.write(&performance_path).unwrap();
    let rejected = temp.path().join("rejected-performance");
    let result = cost(&source, &performance_path, &rejected, mode, 1);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("完整的 correctness 或 probe 计划"));
    assert!(!rejected.exists());
}

/// 诊断对照 D 的准入隔离负例：两份完整 D 输出（进程级真实运行）交给
/// 正式 compare 仍被拒绝——专用诊断结果版本不进正常 evidence 准入，
/// 不能再靠“两份 D 结果一致”得到 case-pass。
#[cfg(not(any(
    feature = "allocation",
    feature = "execution-evidence",
    feature = "motion-kernel-evidence",
    feature = "pose-profiling"
)))]
#[test]
fn diagnostic_outputs_are_rejected_by_formal_compare() {
    let temp = tempfile::tempdir().unwrap();
    let (source, plan, _) = fixture(&temp);
    let mode = if cfg!(feature = "verification-cost-counts") {
        "counts"
    } else {
        "timing"
    };
    let one = temp.path().join("one");
    let two = temp.path().join("two");
    for dir in [&one, &two] {
        let output = cost(&source, &plan, dir, mode, 1);
        assert!(output.status.success());
        let result = read(&dir.join("result.json"));
        assert_eq!(
            result["version"], "urban-result-v9-d-adjacent-diagnostic",
            "D 运行必须携带诊断结果版本"
        );
    }
    let error = compare_runs(&one, &two).unwrap_err();
    assert!(
        error.to_string().contains("cannot accept"),
        "正式 compare 必须拒绝诊断 D 输出：{error}"
    );
}

#[cfg(any(
    feature = "allocation",
    feature = "execution-evidence",
    feature = "motion-kernel-evidence",
    feature = "pose-profiling"
))]
#[test]
fn cost_entry_rejects_unrelated_instrumentation() {
    let temp = tempfile::tempdir().unwrap();
    let (source, plan, _) = fixture(&temp);
    for mode in ["timing", "counts"] {
        let path = temp.path().join(mode);
        let result = cost(&source, &plan, &path, mode, 1);
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("需要"));
        assert!(!path.exists());
    }
}
