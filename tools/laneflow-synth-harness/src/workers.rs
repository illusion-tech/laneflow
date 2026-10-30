use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::Duration;

use laneflow_synth_harness::Error;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const WORKERS: [u32; 5] = [1, 2, 4, 8, 16];
const ROUNDS: u32 = 3;
const MAX_PROCESSES: usize = 4;
const MAX_CONFIGURED_WORKERS: u32 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Job {
    scale: u32,
    case: &'static str,
    dt: u32,
    workers: u32,
    round: u32,
}

impl Job {
    fn identity(self) -> String {
        format!("{}-{}-{}", self.scale, self.case, self.dt)
    }

    fn key(self) -> String {
        format!("{}-w{}-r{}", self.identity(), self.workers, self.round)
    }

    fn baseline(self) -> Self {
        Self {
            workers: 1,
            round: 1,
            ..self
        }
    }

    fn files(self) -> &'static [&'static str] {
        if matches!(self.case, "W3" | "W4") {
            &["ticks.bin", "events.bin", "commands.bin", "batches.json"]
        } else {
            &["ticks.bin", "events.bin"]
        }
    }

    fn windows(self) -> (u64, u64) {
        let warm = if self.dt == 16 { 14_508 } else { 7_044 };
        (warm, if self.case == "W4" { 2 } else { warm * 2 })
    }
}

fn jobs() -> Vec<Job> {
    let mut configs = Vec::new();
    for (scale, dt) in [(10_000, 16), (100_000, 33)] {
        for case in ["W1", "W2", "W3", "W4"] {
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
    for round in 1..=ROUNDS {
        let mut order = configs.clone();
        order.rotate_left((round as usize - 1) * 7);
        jobs.extend(order.into_iter().map(|job| Job { round, ..job }));
    }
    jobs
}

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

fn check_report(
    job: Job,
    report: &Value,
    input: &Value,
    implementation: &str,
) -> Result<(), Error> {
    let identity = &input["identity"];
    let (warm, observation) = job.windows();
    let format = if matches!(job.case, "W3" | "W4") {
        "lf-synth-burst-run-1"
    } else {
        "lf-synth-semantic-run-1"
    };
    if identity["scale"] != job.scale
        || identity["case"] != job.case
        || identity["fixed_delta_ms"] != job.dt
        || report["complete"] != true
        || report["format"] != format
        || report["implementation_commit"] != implementation
        || report["workers"] != job.workers
        || report["input_manifest"] != *input
        || report["warm_up_ticks"] != warm
        || report["observation_ticks"] != observation
        || report["validated_boundaries"] != warm + observation + 1
    {
        return Err(invalid(format!(
            "worker 回执的身份、来源、配置或完整窗口不符：{}",
            job.key()
        )));
    }
    let checkpoints = report["checkpoints"]
        .as_array()
        .ok_or_else(|| invalid(format!("缺少状态检查点：{}", job.key())))?;
    let ticks = if job.case == "W4" {
        vec![0, warm, warm + 1, warm + 2]
    } else {
        vec![
            0,
            warm,
            warm + 1_024,
            warm + 2_048,
            warm + 4_096,
            warm + observation,
        ]
    };
    for tick in ticks {
        if !checkpoints.iter().any(|point| {
            point["tick"] == tick
                && point["state_digest"].as_str().is_some_and(|digest| {
                    digest.len() == 64
                        && digest
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                })
        }) {
            return Err(invalid(format!(
                "状态检查点缺失或非法：{} tick={tick}",
                job.key()
            )));
        }
    }
    Ok(())
}

fn compare(output: &Path, job: Job, baseline: &Value, candidate: &Value) -> Result<Value, Error> {
    let mut a = baseline.clone();
    let mut b = candidate.clone();
    // #1184：worker 是执行配置。只在身份与配置校验后排除这个字段，其他字段全部精确比较。
    a.as_object_mut()
        .ok_or_else(|| invalid("参考回执不是对象"))?
        .remove("workers");
    b.as_object_mut()
        .ok_or_else(|| invalid("候选回执不是对象"))?
        .remove("workers");
    if a != b {
        return Err(invalid(format!("跨 worker 回执语义不同：{}", job.key())));
    }
    for file in job.files() {
        let first = fs::read(output.join(job.baseline().key()).join(file))?;
        let second = fs::read(output.join(job.key()).join(file))?;
        if first != second {
            return Err(invalid(format!(
                "跨 worker 证据字节不同：{}/{file}",
                job.key()
            )));
        }
    }
    Ok(json!({"run":job.key(),"baseline":job.baseline().key(),
        "semantic_report_equal":true,"exact_files":job.files()}))
}

fn file_identity(path: &Path) -> Result<Value, Error> {
    let bytes = fs::read(path)?;
    let digest: String = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(json!({"bytes":bytes.len(),"sha256":digest}))
}

struct Running {
    job: Job,
    child: Child,
}

impl Drop for Running {
    fn drop(&mut self) {
        // 返回 I/O 错误或展开时也收回子进程；正常首错仍先收集已经启动的完整运行。
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn launch(
    binary: &Path,
    input: &Path,
    implementation: &str,
    output: &Path,
    job: Job,
) -> Result<Running, Error> {
    let log = File::create(output.join(format!("{}.log", job.key())))?;
    let child = Command::new(binary)
        .arg("worker-run")
        .arg(input.join(format!("{}-a", job.identity())))
        .arg(implementation)
        .arg(job.workers.to_string())
        .arg(output.join(job.key()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log))
        .spawn()?;
    Ok(Running { job, child })
}

fn collect(
    input: &Path,
    implementation: &str,
    output: &Path,
    job: Job,
    status: ExitStatus,
) -> Result<Value, Error> {
    if !status.success() {
        return Err(invalid(format!(
            "worker 子进程失败 {}：{status}",
            job.key()
        )));
    }
    let target = output.join(job.key());
    let report: Value = serde_json::from_slice(&fs::read(target.join("report.json"))?)?;
    let input: Value = serde_json::from_slice(&fs::read(
        input.join(format!("{}-a/manifest.json", job.identity())),
    )?)?;
    check_report(job, &report, &input, implementation)?;
    let mut files = BTreeMap::new();
    for file in std::iter::once(&"report.json").chain(job.files()) {
        files.insert(*file, file_identity(&target.join(file))?);
    }
    Ok(
        json!({"run":job.key(),"identity":job.identity(),"workers":job.workers,
        "round":job.round,"report":report,"files":files}),
    )
}

fn run_matrix(
    input: &Path,
    implementation: &str,
    environment: &Path,
    output: &Path,
    summary: &mut Value,
) -> Result<(), Error> {
    let environment: Value = serde_json::from_slice(&fs::read(environment)?)?;
    if !environment.is_object()
        || environment["implementation_commit"] != implementation
        || environment["source_clean"] != true
    {
        return Err(invalid("环境声明须为对象，并绑定相同实现提交与干净源码"));
    }
    for field in ["rustc_vv", "cargo_version", "target", "build_command"] {
        if !environment[field]
            .as_str()
            .is_some_and(|value| !value.is_empty())
        {
            return Err(invalid(format!("缺少环境声明字段：{field}")));
        }
    }
    let binary = std::env::current_exe()?;
    summary["binary"] = file_identity(&binary)?;
    summary["environment"] = environment;
    let jobs = jobs();
    let mut running = Vec::<Running>::new();
    let mut completed = BTreeMap::<String, (Job, Value)>::new();
    let mut comparisons = BTreeMap::<String, Value>::new();
    let mut next = 0;
    let mut stopped = false;
    while next < jobs.len() || !running.is_empty() {
        while !stopped
            && running.len() < MAX_PROCESSES
            && next < jobs.len()
            && running.iter().map(|run| run.job.workers).sum::<u32>() + jobs[next].workers
                <= MAX_CONFIGURED_WORKERS
        {
            let job = jobs[next];
            let run = launch(&binary, input, implementation, output, job)?;
            summary["execution_order"].as_array_mut().unwrap().push(json!({
                "run":job.key(),"identity":job.identity(),"workers":job.workers,"round":job.round,"pid":run.child.id()}));
            running.push(run);
            next += 1;
            super::write(&output.join("summary.json"), summary)?;
        }
        let mut progress = false;
        for index in (0..running.len()).rev() {
            let Some(status) = running[index].child.try_wait()? else {
                continue;
            };
            let run = running.swap_remove(index);
            let result = collect(input, implementation, output, run.job, status).and_then(|row| {
                completed.insert(run.job.key(), (run.job, row["report"].clone()));
                summary["runs"].as_array_mut().unwrap().push(row);
                for (key, (job, report)) in &completed {
                    if stopped {
                        break;
                    }
                    if *job == job.baseline() || comparisons.contains_key(key) {
                        continue;
                    }
                    if let Some((_, baseline)) = completed.get(&job.baseline().key()) {
                        comparisons.insert(key.clone(), compare(output, *job, baseline, report)?);
                    }
                }
                Ok(())
            });
            if let Err(error) = result {
                stopped = true;
                summary["errors"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"finished_run":run.job.key(),"error":error.to_string()}));
            }
            summary["comparisons"] = json!(comparisons.values().collect::<Vec<_>>());
            summary["runs"]
                .as_array_mut()
                .unwrap()
                .sort_by(|a, b| a["run"].as_str().cmp(&b["run"].as_str()));
            super::write(&output.join("summary.json"), summary)?;
            eprintln!(
                "worker matrix {}/{} completed, {}/112 compared, {} errors",
                completed.len(),
                jobs.len(),
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
    if stopped || completed.len() != 120 || comparisons.len() != 112 {
        return Err(invalid(
            "worker 矩阵未完整通过；不启动替代轮次，保留全部已运行材料",
        ));
    }
    if file_identity(&binary)? != summary["binary"] {
        return Err(invalid("矩阵执行前后二进制身份发生变化"));
    }
    Ok(())
}

pub(super) fn matrix(
    input: &Path,
    implementation: &str,
    environment: &Path,
    output: &Path,
) -> Result<(), Error> {
    fs::create_dir(output)?;
    let mut summary = json!({"format":"lf-synth-worker-matrix-1","complete":false,
        "scope":"nominal_W1_to_W4_success_path_workers_only_not_performance_or_product_certification",
        "implementation_commit":implementation,"workers":WORKERS,"fresh_process_rounds":ROUNDS,
        "max_concurrent_processes":MAX_PROCESSES,"max_configured_workers":MAX_CONFIGURED_WORKERS,
        "expected_identities":8,"expected_runs":120,"expected_comparisons":112,
        "execution_order":[],"runs":[],"comparisons":[],"errors":[]});
    super::write(&output.join("summary.json"), &summary)?;
    if let Err(error) = run_matrix(input, implementation, environment, output, &mut summary) {
        summary["error"] = Value::String(error.to_string());
        super::write(&output.join("summary.json"), &summary)?;
        return Err(error);
    }
    summary["complete"] = Value::Bool(true);
    super::write(&output.join("summary.json"), &summary)
}

#[cfg(test)]
mod tests;
