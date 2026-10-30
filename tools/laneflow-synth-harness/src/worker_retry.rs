use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::Duration;

use laneflow_synth_harness::Error;
use serde_json::{Value, json};

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
const WORKERS: [u32; 5] = [1, 2, 4, 8, 16];
const MAX_PROCESSES: usize = 4;
const MAX_CONFIGURED_WORKERS: u32 = 32;

fn ensure(valid: bool, message: &str) -> Result<(), Error> {
    if valid {
        Ok(())
    } else {
        Err(Error::Invalid(message.into()))
    }
}

fn jobs() -> Vec<Job> {
    let mut configs = Vec::new();
    for (scale, dt) in [(10_000, 16), (100_000, 33)] {
        for case in ["W3", "W4"] {
            for workers in WORKERS {
                configs.push(Job {
                    scale,
                    case,
                    dt,
                    workers,
                    round: 1,
                });
            }
        }
    }
    let mut jobs = Vec::new();
    for round in 1..=3 {
        let mut order = configs.clone();
        order.rotate_left((round as usize - 1) * 7);
        jobs.extend(order.into_iter().map(|job| Job { round, ..job }));
    }
    jobs
}

fn check_observations(observations: &Value, report: &Value) -> Result<(), Error> {
    ensure(
        observations["format"] == "lf-synth-retry-observations-1",
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
    for (a, b) in [
        ("post-command.lfrs", "failed.lfrs"),
        ("post-command.lfrs", "fresh-post-command.lfrs"),
        ("retry.lfrs", "fresh.lfrs"),
    ] {
        ensure(
            fs::read(target.join(a))? == fs::read(target.join(b))?,
            "同次失败/重试规范快照字节不等",
        )?;
    }
    let observations: Value = serde_json::from_slice(&fs::read(target.join("observations.json"))?)?;
    check_observations(&observations, report)
}

fn compare(output: &Path, job: Job, baseline: &Value, report: &Value) -> Result<Value, Error> {
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
    for file in FILES {
        ensure(
            fs::read(output.join(job.baseline().key()).join(file))?
                == fs::read(output.join(job.key()).join(file))?,
            &format!("跨 worker 失败/重试文件不同：{}/{file}", job.key()),
        )?;
    }
    Ok(
        json!({"run":job.key(),"baseline":job.baseline().key(),"semantic_report_equal":true,"exact_files":FILES}),
    )
}

#[derive(Clone, Copy)]
struct Task {
    job: Job,
    normal: bool,
}

impl Task {
    fn key(self) -> String {
        if self.normal {
            format!("normal-{}", self.job.identity())
        } else {
            self.job.key()
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
        let mut command = Command::new(test_binary);
        command
            .args([
                super::retry::TEST,
                "--exact",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("LF_SYNTH_INPUT", directory)
            .env("LF_SYNTH_OUTPUT", target)
            .env("LF_SYNTH_IMPLEMENTATION", implementation)
            .env("LF_SYNTH_WORKERS", task.job.workers.to_string());
        command
    };
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .spawn()?;
    Ok(Running { task, child })
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
        json!({"run":task.key(),"identity":task.job.identity(),"workers":task.job.workers,"round":task.job.round,"report":report,"files":identities}),
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
        .filter(|job| *job == job.baseline())
        .map(|job| Task { job, normal: true })
        .chain(jobs.iter().copied().map(|job| Task { job, normal: false }))
        .collect();
    let mut normals = BTreeMap::new();
    let mut completed = BTreeMap::<String, (Job, Value)>::new();
    let mut comparisons = BTreeMap::<String, Value>::new();
    let mut running = Vec::<Running>::new();
    let mut next = 0;
    let mut stopped = false;
    while next < tasks.len() || !running.is_empty() {
        while !stopped
            && next < tasks.len()
            && running.len() < MAX_PROCESSES
            && (tasks[next].normal || normals.len() == 4)
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
                    summary["execution_order"].as_array_mut().unwrap().push(json!({"run":task.key(),"normal":task.normal,
                        "workers":task.job.workers,"configured_workers":task.configured_workers(),"round":task.job.round,"pid":run.child.id()}));
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
                        completed.insert(run.task.key(), (run.task.job, row["report"].clone()));
                        summary["runs"].as_array_mut().unwrap().push(row);
                        for (key, (job, report)) in &completed {
                            if stopped {
                                break;
                            }
                            if *job == job.baseline() || comparisons.contains_key(key) {
                                continue;
                            }
                            if let Some((_, baseline)) = completed.get(&job.baseline().key()) {
                                comparisons
                                    .insert(key.clone(), compare(output, *job, baseline, report)?);
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
                    .push(json!({"run":run.task.key(),"error":error.to_string()}));
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
                "worker retry matrix {}/4 normal, {}/60 retry, {}/56 compared, {} errors",
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
        !stopped && normals.len() == 4 && completed.len() == 60 && comparisons.len() == 56,
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
        "scope":"nominal_W3_W4_AfterTransitions_retry_only_not_all_errors_schedules_restore_performance_or_certification",
        "implementation_commit":implementation,"workers":WORKERS,"fresh_process_rounds":3,
        "max_concurrent_processes":MAX_PROCESSES,"max_configured_workers_including_fresh_worlds":MAX_CONFIGURED_WORKERS,
        "expected_normal_runs":4,"expected_runs":60,"expected_comparisons":56,"execution_order":[],"normal_runs":[],"runs":[],"comparisons":[],"errors":[]});
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
