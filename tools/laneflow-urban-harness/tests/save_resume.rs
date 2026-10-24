//! 存档与续跑（设计 §7）：续跑等价、分段比较与恢复拒绝面。

use std::{
    fs,
    path::{Path, PathBuf},
};

use laneflow_urban_generator::{Scale, UrbanConfig, generate};
use laneflow_urban_harness::{
    Artifacts, Diagnostics, ResolvedPlan, RunOptions, RunResult, UrbanCase, Window, compare_runs,
    run_with_options,
};
use serde_json::Value;

/// 与 harness 文件字节摘要相同的独立计算（BLAKE3-256，域分隔前缀见设计 §6）。
fn file_digest(bytes: &[u8]) -> String {
    let mut preimage = b"laneflow:urban-harness-file:v1\0".to_vec();
    preimage.extend_from_slice(bytes);
    blake3::hash(&preimage).to_hex().to_string()
}

fn execution(workers: u32) -> laneflow_runtime::ExecutionConfig {
    laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::new(workers).unwrap())
}

fn artifacts(temp: &tempfile::TempDir) -> Artifacts {
    let config =
        UrbanConfig::parse(include_str!("../../../examples/config/cn-urban.toml")).unwrap();
    let source = temp.path().join("source");
    generate(&config, Scale::Fixture, &source, None).unwrap();
    Artifacts::load(&source).unwrap()
}

fn run(
    artifacts: &Artifacts,
    plan: &ResolvedPlan,
    output: &Path,
    workers: u32,
    options: RunOptions,
) -> laneflow_urban_harness::Result<RunResult> {
    run_with_options(
        artifacts,
        plan,
        output,
        execution(workers),
        Diagnostics::Enabled,
        &options,
    )
}

fn save_dir(output: &Path, tick: u64) -> PathBuf {
    output.join("saves").join(format!("tick-{tick}"))
}

/// 把存档复制到新位置，按 `edit` 改写 caller.json，并重算清单里的摘要，
/// 模拟「文件完整但内容错误」的存档（本程序序列化缺陷的形态）。
fn tampered(save: &Path, target: &Path, edit: impl FnOnce(&mut Value)) -> PathBuf {
    fs::create_dir_all(target).unwrap();
    for name in ["world.lfrs", "save.toml"] {
        fs::copy(save.join(name), target.join(name)).unwrap();
    }
    let mut caller: Value =
        serde_json::from_slice(&fs::read(save.join("caller.json")).unwrap()).unwrap();
    edit(&mut caller);
    let bytes = serde_json::to_vec(&caller).unwrap();
    fs::write(target.join("caller.json"), &bytes).unwrap();
    let mut manifest: toml::Table =
        toml::from_str(&fs::read_to_string(target.join("save.toml")).unwrap()).unwrap();
    let caller_digest = manifest["caller"].as_table_mut().unwrap();
    caller_digest.insert("bytes".into(), toml::Value::Integer(bytes.len() as i64));
    caller_digest.insert("digest".into(), toml::Value::String(file_digest(&bytes)));
    fs::write(
        target.join("save.toml"),
        toml::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
    target.to_path_buf()
}

#[test]
fn resumed_runs_match_the_uninterrupted_run_and_bind_their_start() {
    let temp = tempfile::tempdir().unwrap();
    let artifacts = artifacts(&temp);
    let plan = ResolvedPlan::for_case(
        &artifacts,
        UrbanCase::SustainedActive,
        Window::probe_after(32, 768).unwrap(),
    )
    .unwrap();
    let end = plan.window.end();
    let full = temp.path().join("full");
    let full_result = run(
        &artifacts,
        &plan,
        &full,
        1,
        RunOptions {
            save_at: vec![16, 32, 400, 700],
            ..RunOptions::default()
        },
    )
    .unwrap();
    // 非 performance 计划带存档仍是正式运行。
    assert_eq!(
        full_result.status, "probe-complete",
        "{:?}",
        full_result.error
    );
    assert_eq!(full_result.saves.len(), 4);
    assert!(full_result.replacements > 0);
    // 账本只存在途请求：结束的请求已移出，登记项恰是排队队列。
    let caller: Value =
        serde_json::from_slice(&fs::read(save_dir(&full, 700).join("caller.json")).unwrap())
            .unwrap();
    let ledger = &caller["ledger"];
    assert_eq!(
        ledger["entries"].as_array().unwrap().len(),
        caller["schedule"].as_array().unwrap().len()
    );
    assert!(
        ledger["next_sequence"].as_u64().unwrap()
            > ledger["entries"].as_array().unwrap().len() as u64
    );

    for tick in [16, 32, 400, 700] {
        // 1 worker 存档、4 worker 续跑。
        let resumed = temp.path().join(format!("resume-{tick}"));
        let result = run(
            &artifacts,
            &plan,
            &resumed,
            4,
            RunOptions {
                resume: Some(save_dir(&full, tick)),
                ..RunOptions::default()
            },
        )
        .unwrap();
        assert_eq!(result.status, "segment-complete", "{:?}", result.error);
        assert_eq!(result.start_tick(), tick);
        assert_eq!(result.endpoint, full_result.endpoint);
        assert_eq!(result.tile_evidence, full_result.tile_evidence);
        assert_eq!(
            fs::read_to_string(resumed.join("ticks.jsonl"))
                .unwrap()
                .lines()
                .count() as u64,
            end - tick
        );
        let comparison = compare_runs(&full, &resumed).unwrap();
        assert_eq!(comparison.version, "urban-comparison-v4");
        assert_eq!(comparison.status, "segment-match");
        assert_eq!(comparison.interval, (tick, end));
        assert_eq!(comparison.right.start, tick);
    }

    // 续跑可以再存档；两份从同一存档续跑的运行可直接比较。
    let first = temp.path().join("chain-first");
    run(
        &artifacts,
        &plan,
        &first,
        4,
        RunOptions {
            resume: Some(save_dir(&full, 32)),
            save_at: vec![500],
            until_tick: Some(600),
        },
    )
    .unwrap();
    let second = temp.path().join("chain-second");
    run(
        &artifacts,
        &plan,
        &second,
        1,
        RunOptions {
            resume: Some(save_dir(&first, 500)),
            until_tick: Some(600),
            ..RunOptions::default()
        },
    )
    .unwrap();
    assert_eq!(compare_runs(&first, &second).unwrap().interval, (500, 600));
    // 第二次运行的存档来自 first，而不是 full：full 只到计划终点，终点不同也拒绝。
    assert!(compare_runs(&full, &second).is_err());

    // 两个从第 0 拍起跑的分段：比较初始检查点。
    let fresh_a = temp.path().join("fresh-a");
    let fresh_b = temp.path().join("fresh-b");
    for (output, workers) in [(&fresh_a, 1), (&fresh_b, 4)] {
        let result = run(
            &artifacts,
            &plan,
            output,
            workers,
            RunOptions {
                until_tick: Some(300),
                ..RunOptions::default()
            },
        )
        .unwrap();
        assert_eq!(result.status, "segment-complete");
    }
    let comparison = compare_runs(&fresh_a, &fresh_b).unwrap();
    assert_eq!(
        (comparison.status.as_str(), comparison.interval),
        ("segment-match", (0, 300))
    );

    // 用同计划另一次执行写出的存档续跑：存档不在从第 0 拍起跑的运行的 saves 中，拒绝比较。
    let other = temp.path().join("other");
    run(
        &artifacts,
        &plan,
        &other,
        1,
        RunOptions {
            save_at: vec![400],
            until_tick: Some(400),
            ..RunOptions::default()
        },
    )
    .unwrap();
    let foreign = temp.path().join("foreign");
    run(
        &artifacts,
        &plan,
        &foreign,
        1,
        RunOptions {
            resume: Some(save_dir(&other, 400)),
            ..RunOptions::default()
        },
    )
    .unwrap();
    assert!(
        compare_runs(&full, &foreign)
            .unwrap_err()
            .to_string()
            .contains("same state")
    );

    // 缺少区间内计划规定的检查点（暖机末第 32 拍落在 (16, end] 内）：拒绝比较。
    let resumed = temp.path().join("resume-16");
    let mut receipt: Value =
        serde_json::from_slice(&fs::read(resumed.join("result.json")).unwrap()).unwrap();
    let warm = plan.window.warm_up_ticks.to_string();
    assert!(
        receipt["checkpoints"]
            .as_object_mut()
            .unwrap()
            .remove(&warm)
            .is_some()
    );
    fs::write(
        resumed.join("result.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
    assert!(
        compare_runs(&full, &resumed)
            .unwrap_err()
            .to_string()
            .contains("planned checkpoint")
    );
}

#[test]
fn resume_rejects_malformed_saves_without_writing_output() {
    let temp = tempfile::tempdir().unwrap();
    let artifacts = artifacts(&temp);
    let plan = ResolvedPlan::for_case(
        &artifacts,
        UrbanCase::SustainedActive,
        Window::probe_after(16, 784).unwrap(),
    )
    .unwrap();
    let full = temp.path().join("full");
    run(
        &artifacts,
        &plan,
        &full,
        1,
        RunOptions {
            save_at: vec![600],
            ..RunOptions::default()
        },
    )
    .unwrap();
    let save = save_dir(&full, 600);
    // Mixed 计划在存档时仍有排在后面的出发请求，用来篡改请求队列与登记表。
    let mixed_plan = ResolvedPlan::mixed(
        &artifacts,
        Window {
            purpose: "probe".into(),
            warm_up_ticks: 0,
            observation_ticks: 64,
        },
    )
    .unwrap();
    let mixed = temp.path().join("mixed");
    run(
        &artifacts,
        &mixed_plan,
        &mixed,
        1,
        RunOptions {
            save_at: vec![4],
            ..RunOptions::default()
        },
    )
    .unwrap();
    let mixed_save = save_dir(&mixed, 4);
    let sustained = (&plan, save.as_path());
    let queued = (&mixed_plan, mixed_save.as_path());
    let mut index = 0;
    let mut reject = |(plan, save): (&ResolvedPlan, &Path),
                      label: &str,
                      edit: &dyn Fn(&mut Value),
                      expected: &str| {
        index += 1;
        let broken = tampered(save, &temp.path().join(format!("broken-{index}")), edit);
        let output = temp.path().join(format!("rejected-{index}"));
        let error = run(
            &artifacts,
            plan,
            &output,
            1,
            RunOptions {
                resume: Some(broken),
                ..RunOptions::default()
            },
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains(expected), "{label}: {error}");
        assert!(!output.exists(), "{label}: rejected resume wrote output");
    };
    reject(
        sustained,
        "truncated tile evidence",
        &|caller| {
            caller["evidence"].as_array_mut().unwrap().pop();
        },
        "tile evidence",
    );
    reject(
        sustained,
        "overlong tile evidence",
        &|caller| {
            let evidence = caller["evidence"].as_array_mut().unwrap();
            evidence.push(evidence[0].clone());
        },
        "tile evidence",
    );
    reject(
        sustained,
        "duplicated route id",
        &|caller| {
            let routes = caller["routes"].as_array_mut().unwrap();
            routes[1]["snapshot_route_id"] = routes[0]["snapshot_route_id"].clone();
        },
        "route bindings",
    );
    reject(
        sustained,
        "duplicated vehicle id",
        &|caller| {
            let individuals = caller["individuals"].as_array_mut().unwrap();
            individuals[1]["vehicle"] = individuals[0]["vehicle"].clone();
        },
        "vehicle bindings",
    );
    let caller: Value =
        serde_json::from_slice(&fs::read(mixed_save.join("caller.json")).unwrap()).unwrap();
    assert!(
        !caller["schedule"].as_array().unwrap().is_empty(),
        "the save must hold queued requests"
    );
    reject(
        queued,
        "dropped queued request",
        &|caller| {
            caller["schedule"].as_array_mut().unwrap().pop();
        },
        "disagree",
    );
    reject(
        queued,
        "moved ledger retry tick",
        &|caller| {
            let entries = caller["ledger"]["entries"].as_array_mut().unwrap();
            let state = entries
                .iter_mut()
                .map(|entry| &mut entry["state"])
                .find(|state| state["state"] == "scheduled" || state["next_tick"].is_u64())
                .expect("a queued request");
            let field = if state["state"] == "scheduled" {
                "due"
            } else {
                "next_tick"
            };
            let tick = state[field].as_u64().unwrap();
            state[field] = (tick + 1).into();
        },
        "disagree",
    );
    reject(
        sustained,
        "recycling owner without request",
        &|caller| {
            let pending = caller["recycling"].as_array_mut().unwrap();
            let slot = pending.iter().position(Value::is_null).unwrap();
            pending[slot] = 999_999.into();
        },
        "recycling owners",
    );
    reject(
        sustained,
        "swapped routes",
        &|caller| {
            let routes = caller["routes"].as_array_mut().unwrap();
            let last = routes.len() - 1;
            let first = routes[0]["snapshot_route_id"].clone();
            routes[0]["snapshot_route_id"] = routes[last]["snapshot_route_id"].clone();
            routes[last]["snapshot_route_id"] = first;
        },
        "",
    );
    // 模拟导出缺陷：结构与世界摘要都不覆盖的宿主字段被写错，由宿主状态摘要往返比对发现。
    reject(
        sustained,
        "caller field written wrong",
        &|caller| {
            let individual = &mut caller["individuals"][0];
            individual["crossed_tile"] = (!individual["crossed_tile"].as_bool().unwrap()).into();
        },
        "caller state digest",
    );

    // 区间与版本：在恢复世界之前拒绝。
    let resume = |options: RunOptions, label: &str, expected: &str| {
        let output = temp.path().join(format!("interval-{label}"));
        let error = run(&artifacts, &plan, &output, 1, options)
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "{label}: {error}");
        assert!(!output.exists(), "{label}");
    };
    resume(
        RunOptions {
            resume: Some(save.clone()),
            until_tick: Some(600),
            ..RunOptions::default()
        },
        "until-at-save",
        "T < E",
    );
    resume(
        RunOptions {
            resume: Some(save.clone()),
            save_at: vec![600],
            ..RunOptions::default()
        },
        "save-before-start",
        "--save-at",
    );
    resume(
        RunOptions {
            save_at: vec![100],
            until_tick: Some(80),
            ..RunOptions::default()
        },
        "save-after-end",
        "--save-at",
    );
    resume(
        RunOptions {
            until_tick: Some(0),
            ..RunOptions::default()
        },
        "until-zero",
        "--until-tick",
    );
    let other_plan = ResolvedPlan::for_case(
        &artifacts,
        UrbanCase::SustainedActive,
        Window::probe_after(16, 800).unwrap(),
    )
    .unwrap();
    let output = temp.path().join("other-plan");
    assert!(
        run(
            &artifacts,
            &other_plan,
            &output,
            1,
            RunOptions {
                resume: Some(save.clone()),
                ..RunOptions::default()
            },
        )
        .unwrap_err()
        .to_string()
        .contains("different resolved plan")
    );
    let wrong_version = temp.path().join("wrong-version");
    fs::create_dir_all(&wrong_version).unwrap();
    for name in ["world.lfrs", "caller.json"] {
        fs::copy(save.join(name), wrong_version.join(name)).unwrap();
    }
    fs::write(
        wrong_version.join("save.toml"),
        fs::read_to_string(save.join("save.toml"))
            .unwrap()
            .replace("urban-observation-v4", "urban-observation-v3"),
    )
    .unwrap();
    resume(
        RunOptions {
            resume: Some(wrong_version),
            ..RunOptions::default()
        },
        "observation-version",
        "observation encoding",
    );
    // `urban-save-v2` 为 SHA-256 文件摘要的上一版清单（#1042）：不读取、不转换。
    let old_save = temp.path().join("old-save-version");
    fs::create_dir_all(&old_save).unwrap();
    for name in ["world.lfrs", "caller.json"] {
        fs::copy(save.join(name), old_save.join(name)).unwrap();
    }
    fs::write(
        old_save.join("save.toml"),
        fs::read_to_string(save.join("save.toml"))
            .unwrap()
            .replace("urban-save-v3", "urban-save-v2"),
    )
    .unwrap();
    resume(
        RunOptions {
            resume: Some(old_save),
            ..RunOptions::default()
        },
        "old-save-version",
        "unsupported save version",
    );
}
