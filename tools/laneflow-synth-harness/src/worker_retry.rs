use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::Duration;

use laneflow_synth_harness::Error;
use serde_json::{Value, json};

use super::retry::TEST;
use super::workers::{Job, check_report, file_identity};

const FILES: [&str; 8] = [
    "pre-command.lfrs",
    "post-command.lfrs",
    "fresh-post-command.lfrs",
    "failed.lfrs",
    "retry.lfrs",
    "fresh.lfrs",
    "final.lfrs",
    "observations.json",
];
/// L 族（#1218）：T 触时刻（retry/fresh/final）按面 B 记录（结构化拒绝变体 +
/// 真实资源身份/历史投影）代替规范快照；T-free 边界四份 `.lfrs` 逐字节不变。
const L_FILES: [&str; 8] = [
    "pre-command.lfrs",
    "post-command.lfrs",
    "fresh-post-command.lfrs",
    "failed.lfrs",
    "retry.json",
    "fresh.json",
    "final.json",
    "observations.json",
];
const L_TEST: &str = "kernel::lf_synth_contract::lane_change_retry::lane_change_failed_step_retry";
const L_FORMAT: &str = "lf-synth-lane-change-retry-1";
const WORKERS: [u32; 5] = [1, 2, 4, 8, 16];
const MAX_PROCESSES: usize = 4;
const MAX_CONFIGURED_WORKERS: u32 = 32;
/// L 族注入断点（#1218 D6）：换道预约相关断点。
const L_FAILPOINTS: [&str; 2] = ["AfterGrants", "AfterTransitions"];

fn ensure(valid: bool, message: &str) -> Result<(), Error> {
    if valid {
        Ok(())
    } else {
        Err(Error::Invalid(message.into()))
    }
}

/// 矩阵行 = （身份 job，注入断点）；W 族断点恒空（既有 AfterTransitions 固定注入
/// 由运行侧驱动承担）。
fn jobs() -> Vec<(Job, &'static str)> {
    let mut configs = Vec::new();
    for (scale, dt) in [(10_000, 16), (100_000, 33)] {
        for case in ["W3", "W4"] {
            for workers in WORKERS {
                configs.push((
                    Job {
                        scale,
                        case,
                        dt,
                        workers,
                        round: 1,
                    },
                    "",
                ));
            }
        }
        for inject in L_FAILPOINTS {
            for workers in WORKERS {
                configs.push((
                    Job {
                        scale,
                        case: "L1",
                        dt,
                        workers,
                        round: 1,
                    },
                    inject,
                ));
            }
        }
    }
    let mut jobs = Vec::new();
    for round in 1..=3 {
        let mut order = configs.clone();
        order.rotate_left((round as usize - 1) * 7);
        jobs.extend(
            order
                .into_iter()
                .map(|(job, inject)| (Job { round, ..job }, inject)),
        );
    }
    jobs
}

fn check_observations(observations: &Value, report: &Value) -> Result<(), Error> {
    let lane_change = report["format"] == L_FORMAT;
    ensure(
        observations["format"]
            == if lane_change {
                "lf-synth-retry-observations-lc-1"
            } else {
                "lf-synth-retry-observations-1"
            },
        "缺少规范边界观测",
    )?;
    let boundary = report["failure_boundary"]
        .as_u64()
        .ok_or_else(|| Error::Invalid("缺少失败边界".into()))?;
    let dt = report["input_manifest"]["identity"]["fixed_delta_ms"]
        .as_u64()
        .ok_or_else(|| Error::Invalid("缺少步长".into()))?;
    for (name, tick, cursor, outcome) in [
        (
            "pre_command",
            boundary,
            &report["cursor_before_commands"],
            false,
        ),
        (
            "post_command",
            boundary,
            &report["cursor_after_commands"],
            false,
        ),
        (
            "fresh_post_command",
            boundary,
            &report["cursor_after_commands"],
            false,
        ),
        ("failed", boundary, &report["cursor_after_commands"], false),
        ("retry", boundary + 1, &report["cursor_after_retry"], true),
        ("fresh", boundary + 1, &report["cursor_after_retry"], true),
        (
            "final",
            report["final_tick"].as_u64().unwrap_or(0),
            &report["final_cursor"],
            true,
        ),
    ] {
        let value = &observations[name];
        ensure(
            value["tick"] == tick
                && value["time_ms"] == tick * dt
                && cursor.is_u64()
                && value["command_cursor"] == *cursor,
            "边界观测时间或游标不符",
        )?;
        for (field, width) in [("events", 11), ("signals", 2)] {
            ensure(
                value[field].as_array().is_some_and(|rows| {
                    rows.iter().all(|row| {
                        row.as_array().is_some_and(|cells| {
                            cells.len() == width && cells.iter().all(Value::is_u64)
                        })
                    })
                }),
                "缺少完整事件或信号字段",
            )?;
        }
        if outcome {
            let observed = &value["outcome"];
            ensure(
                observed["tick"] == tick
                    && observed["time_ms"] == tick * dt
                    && observed["parking_arrivals"].as_array().is_some_and(|rows| {
                        rows.iter().all(|row| {
                            row.as_array().is_some_and(|cells| {
                                cells.len() == 2 && cells.iter().all(Value::is_u64)
                            })
                        })
                    }),
                "缺少完整步进结果",
            )?;
        } else {
            ensure(value["outcome"].is_null(), "失败边界不得伪造成功结果")?;
        }
    }
    ensure(
        observations["post_command"] == observations["failed"]
            && observations["post_command"] == observations["fresh_post_command"]
            && observations["retry"] == observations["fresh"],
        "失败零发布或 retry/fresh 边界观测不等",
    )
}

fn check_local_files(target: &Path, report: &Value) -> Result<(), Error> {
    let lane_change = report["format"] == L_FORMAT;
    let equal_pairs: &[(&str, &str)] = if lane_change {
        &[
            ("post-command.lfrs", "failed.lfrs"),
            ("post-command.lfrs", "fresh-post-command.lfrs"),
            ("retry.json", "fresh.json"),
        ]
    } else {
        &[
            ("post-command.lfrs", "failed.lfrs"),
            ("post-command.lfrs", "fresh-post-command.lfrs"),
            ("retry.lfrs", "fresh.lfrs"),
        ]
    };
    for (a, b) in equal_pairs {
        ensure(
            fs::read(target.join(a))? == fs::read(target.join(b))?,
            &format!("同次失败/重试记录不等：{a}/{b}"),
        )?;
    }
    if lane_change {
        // 面 B（#1218/#1217）：T 触时刻记录必须是结构化拒绝 + 真实身份/历史投影。
        for name in ["retry.json", "fresh.json", "final.json"] {
            let value: Value = serde_json::from_slice(&fs::read(target.join(name))?)?;
            ensure(
                value["persistence_refused"].is_string()
                    && value["journal"].is_array()
                    && value["vehicles"].is_array(),
                "面 B 记录缺拒绝变体或真实身份投影",
            )?;
        }
    }
    let observations: Value = serde_json::from_slice(&fs::read(target.join("observations.json"))?)?;
    check_observations(&observations, report)
}

fn compare(output: &Path, task: Task, baseline: &Value, report: &Value) -> Result<Value, Error> {
    let mut a = baseline.clone();
    let mut b = report.clone();
    // #1191：来源、输入、配置和 guardrail 已先校验，仅排除真实执行配置。
    a.as_object_mut()
        .ok_or_else(|| Error::Invalid("参考回执不是对象".into()))?
        .remove("workers");
    b.as_object_mut()
        .ok_or_else(|| Error::Invalid("候选回执不是对象".into()))?
        .remove("workers");
    ensure(a == b, "跨 worker 失败/重试语义回执不同")?;
    let files: &[&str] = if task.job.case == "L1" {
        &L_FILES
    } else {
        &FILES
    };
    for file in files {
        ensure(
            fs::read(output.join(task.baseline_key()).join(file))?
                == fs::read(output.join(task.key()).join(file))?,
            &format!("跨 worker 失败/重试文件不同：{}/{file}", task.key()),
        )?;
    }
    Ok(json!({"run":task.key(),"baseline":task.baseline_key(),
        "semantic_report_equal":true,"exact_files":files}))
}

#[derive(Clone, Copy)]
struct Task {
    job: Job,
    normal: bool,
    inject: &'static str,
}

impl Task {
    fn key(self) -> String {
        if self.normal {
            format!("normal-{}", self.job.identity())
        } else if self.inject.is_empty() {
            self.job.key()
        } else {
            format!("{}-{}", self.inject, self.job.key())
        }
    }

    fn baseline_key(self) -> String {
        let base = self.job.baseline();
        if self.inject.is_empty() {
            base.key()
        } else {
            format!("{}-{}", self.inject, base.key())
        }
    }

    fn configured_workers(self) -> u32 {
        if self.normal { 1 } else { self.job.workers * 2 }
    }
}

struct Running {
    task: Task,
    child: Child,
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn launch(
    binary: &Path,
    test_binary: &Path,
    input: &Path,
    implementation: &str,
    output: &Path,
    task: Task,
) -> Result<Running, Error> {
    let log = File::create(output.join(format!("{}.log", task.key())))?;
    let directory = input.join(format!("{}-a", task.job.identity()));
    let target = output.join(task.key());
    let mut command = if task.normal {
        let mut command = Command::new(binary);
        command
            .arg("worker-run")
            .arg(directory)
            .arg(implementation)
            .arg("1")
            .arg(target);
        command
    } else {
        let test = if task.job.case == "L1" { L_TEST } else { TEST };
        let mut command = Command::new(test_binary);
        command
            .args([
                test,
                "--exact",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("LF_SYNTH_INPUT", directory)
            .env("LF_SYNTH_OUTPUT", target)
            .env("LF_SYNTH_IMPLEMENTATION", implementation)
            .env("LF_SYNTH_WORKERS", task.job.workers.to_string());
        if task.job.case == "L1" {
            command.env("LF_SYNTH_FAILPOINT", task.inject);
        }
        command
    };
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .spawn()?;
    Ok(Running { task, child })
}

/// L 族同源正常参考核对（#1218）：末态面 B 历史（真实资源身份）与正常参考终检查点
/// 的 `t_state.journal` 相等；失败边界/窗口/来源一致。
fn verify_lane_change_reports(
    report: &Value,
    normal: &Value,
    identity: &str,
    workers: u32,
) -> Result<(), Error> {
    ensure(
        report["format"] == L_FORMAT
            && report["complete"] == true
            && report["implementation_commit"] == normal["implementation_commit"]
            && report["input_manifest"] == normal["input_manifest"]
            && report["warm_up_ticks"] == normal["warm_up_ticks"]
            && report["observation_ticks"] == normal["observation_ticks"]
            && report["workers"] == workers
            && report["zero_publication"] == true
            && report["retry_equals_fresh"] == true
            && report["failpoint_consumed"] == true,
        &format!("换道族失败重试回执来源/配置/零发布不符：{identity}"),
    )?;
    let points = normal["checkpoints"]
        .as_array()
        .ok_or_else(|| Error::Invalid("正常参考缺检查点".into()))?;
    let final_point = points
        .last()
        .ok_or_else(|| Error::Invalid("正常参考缺终检查点".into()))?;
    ensure(
        final_point["persistence_refused"].is_string()
            && final_point["t_state"]["journal"] == report["final_face_b"]["journal"],
        &format!("换道族末态面 B 历史与同源正常参考不符：{identity}"),
    )
}

fn collect(
    input: &Path,
    implementation: &str,
    output: &Path,
    task: Task,
    status: ExitStatus,
    normals: &BTreeMap<String, Value>,
) -> Result<Value, Error> {
    ensure(
        status.success(),
        &format!("子进程失败 {}：{status}", task.key()),
    )?;
    let target = output.join(task.key());
    let report: Value = serde_json::from_slice(&fs::read(target.join("report.json"))?)?;
    let files: &[&str] = if task.normal {
        let input: Value = serde_json::from_slice(&fs::read(
            input.join(format!("{}-a/manifest.json", task.job.identity())),
        )?)?;
        check_report(task.job, &report, &input, implementation)?;
        task.job.files()
    } else if task.job.case == "L1" {
        let normal = normals
            .get(&task.job.identity())
            .ok_or_else(|| Error::Invalid("缺少同源完整正常参考".into()))?;
        verify_lane_change_reports(&report, normal, &task.job.identity(), task.job.workers)?;
        check_local_files(&target, &report)?;
        &L_FILES
    } else {
        let normal = normals
            .get(&task.job.identity())
            .ok_or_else(|| Error::Invalid("缺少同源完整正常参考".into()))?;
        super::retry::verify_reports(
            &report,
            normal,
            implementation,
            &task.job.identity(),
            task.job.workers,
        )?;
        check_local_files(&target, &report)?;
        &FILES
    };
    let mut identities = BTreeMap::new();
    for file in std::iter::once(&"report.json").chain(files) {
        identities.insert(*file, file_identity(&target.join(file))?);
    }
    Ok(
        json!({"run":task.key(),"identity":task.job.identity(),"inject":task.inject,
        "workers":task.job.workers,"round":task.job.round,"report":report,"files":identities}),
    )
}

fn run_matrix(
    test_binary: &Path,
    input: &Path,
    implementation: &str,
    environment: &Path,
    output: &Path,
    summary: &mut Value,
) -> Result<(), Error> {
    ensure(
        implementation.len() == 40
            && implementation
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
        "实现来源必须是完整提交 ID",
    )?;
    let environment: Value = serde_json::from_slice(&fs::read(environment)?)?;
    ensure(
        environment.is_object()
            && environment["implementation_commit"] == implementation
            && environment["source_clean"] == true,
        "环境须绑定相同实现提交与干净源码",
    )?;
    for field in [
        "rustc_vv",
        "cargo_version",
        "target",
        "build_command",
        "test_build_command",
    ] {
        ensure(
            environment[field].as_str().is_some_and(|s| !s.is_empty()),
            &format!("缺少环境字段：{field}"),
        )?;
    }
    let binary = std::env::current_exe()?;
    summary["binary"] = file_identity(&binary)?;
    summary["test_binary"] = file_identity(test_binary)?;
    summary["environment"] = environment;
    let jobs = jobs();
    let tasks: Vec<_> = jobs
        .iter()
        .copied()
        .filter(|(job, inject)| job.workers == 1 && job.round == 1 && inject.is_empty())
        .map(|(job, _)| Task {
            job,
            normal: true,
            inject: "",
        })
        .chain(jobs.iter().copied().map(|(job, inject)| Task {
            job,
            normal: false,
            inject,
        }))
        .collect();
    let mut normals = BTreeMap::new();
    let mut completed = BTreeMap::<String, (Task, Value)>::new();
    let mut comparisons = BTreeMap::<String, Value>::new();
    let mut running = Vec::<Running>::new();
    let mut next = 0;
    let mut stopped = false;
    while next < tasks.len() || !running.is_empty() {
        while !stopped
            && next < tasks.len()
            && running.len() < MAX_PROCESSES
            && (tasks[next].normal || normals.len() == 6)
            && running
                .iter()
                .map(|run| run.task.configured_workers())
                .sum::<u32>()
                + tasks[next].configured_workers()
                <= MAX_CONFIGURED_WORKERS
        {
            let task = tasks[next];
            match launch(&binary, test_binary, input, implementation, output, task) {
                Ok(run) => {
                    summary["execution_order"].as_array_mut().unwrap().push(
                        json!({"run":task.key(),
                        "normal":task.normal,"inject":task.inject,
                        "workers":task.job.workers,"configured_workers":task.configured_workers(),
                        "round":task.job.round,"pid":run.child.id()}),
                    );
                    running.push(run);
                    next += 1;
                    super::write(&output.join("summary.json"), summary)?;
                }
                Err(error) => {
                    stopped = true;
                    summary["errors"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!({"run":task.key(),"error":error.to_string()}));
                }
            }
        }
        let mut progress = false;
        for index in (0..running.len()).rev() {
            let Some(status) = running[index].child.try_wait()? else {
                continue;
            };
            let run = running.swap_remove(index);
            let result = collect(input, implementation, output, run.task, status, &normals)
                .and_then(|row| {
                    if run.task.normal {
                        normals.insert(run.task.job.identity(), row["report"].clone());
                        summary["normal_runs"].as_array_mut().unwrap().push(row);
                    } else {
                        completed.insert(run.task.key(), (run.task, row["report"].clone()));
                        summary["runs"].as_array_mut().unwrap().push(row);
                        for (key, (task, report)) in &completed {
                            if stopped {
                                break;
                            }
                            if key == &task.baseline_key() || comparisons.contains_key(key) {
                                continue;
                            }
                            if let Some((_, baseline)) = completed.get(&task.baseline_key()) {
                                comparisons
                                    .insert(key.clone(), compare(output, *task, baseline, report)?);
                            }
                        }
                    }
                    Ok(())
                });
            if let Err(error) = result {
                stopped = true;
                summary["errors"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"finished_run":run.task.key(),"error":error.to_string()}));
            }
            summary["comparisons"] = json!(comparisons.values().collect::<Vec<_>>());
            for field in ["normal_runs", "runs"] {
                summary[field]
                    .as_array_mut()
                    .unwrap()
                    .sort_by(|a, b| a["run"].as_str().cmp(&b["run"].as_str()));
            }
            super::write(&output.join("summary.json"), summary)?;
            eprintln!(
                "worker retry matrix {}/6 normal, {}/120 retry, {}/112 compared, {} errors",
                normals.len(),
                completed.len(),
                comparisons.len(),
                summary["errors"].as_array().unwrap().len()
            );
            progress = true;
        }
        if stopped && running.is_empty() {
            break;
        }
        if !progress {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    ensure(
        !stopped && normals.len() == 6 && completed.len() == 120 && comparisons.len() == 112,
        "F2 矩阵未完整通过；保留原始材料，不补跑拼接",
    )?;
    ensure(
        file_identity(&binary)? == summary["binary"]
            && file_identity(test_binary)? == summary["test_binary"],
        "执行前后二进制身份发生变化",
    )
}

pub(super) fn matrix(
    test_binary: &Path,
    input: &Path,
    implementation: &str,
    environment: &Path,
    output: &Path,
) -> Result<(), Error> {
    fs::create_dir(output)?;
    let mut summary = json!({"format":"lf-synth-worker-retry-matrix-1","complete":false,
        "scope":"nominal_W3_W4_AfterTransitions_and_L1_AfterGrants_AfterTransitions_retry_only_not_all_errors_schedules_restore_performance_or_certification",
        "implementation_commit":implementation,"workers":WORKERS,"fresh_process_rounds":3,
        "lane_change_failpoints":L_FAILPOINTS,
        "max_concurrent_processes":MAX_PROCESSES,"max_configured_workers_including_fresh_worlds":MAX_CONFIGURED_WORKERS,
        "expected_normal_runs":6,"expected_runs":120,"expected_comparisons":112,
        "execution_order":[],"normal_runs":[],"runs":[],"comparisons":[],"errors":[]});
    super::write(&output.join("summary.json"), &summary)?;
    if let Err(error) = run_matrix(
        test_binary,
        input,
        implementation,
        environment,
        output,
        &mut summary,
    ) {
        summary["error"] = json!(error.to_string());
        super::write(&output.join("summary.json"), &summary)?;
        return Err(error);
    }
    summary["complete"] = json!(true);
    super::write(&output.join("summary.json"), &summary)
}

#[cfg(test)]
mod tests;
