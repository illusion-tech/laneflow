use std::ffi::OsString;
use std::fs::{self, File};
use std::num::NonZeroU32;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::workers::Job;
use laneflow_runtime::ExecutionConfig;
use laneflow_synth_harness::{Error, restore};
use serde_json::{Value, json};

#[derive(Clone, Copy)]
struct Task {
    job: Job,
    checkpoint: Option<usize>,
}
impl Task {
    fn key(self) -> String {
        match self.checkpoint {
            None => format!("{}-reference", self.job.identity()),
            Some(i) => format!("{}-c{i}", self.job.key()),
        }
    }
    fn reference(self) -> String {
        format!("{}-reference", self.job.identity())
    }
}

fn tasks(probe: bool, references: bool) -> Vec<Task> {
    let mut configs = Vec::new();
    for (scale, dt) in [(10_000, 16), (100_000, 33)] {
        for case in ["W1", "W2", "W3", "W4"] {
            if references {
                configs.push(Task {
                    job: Job {
                        scale,
                        dt,
                        case,
                        workers: 1,
                        round: 1,
                    },
                    checkpoint: None,
                });
            } else {
                for workers in if probe {
                    vec![1, 16]
                } else {
                    vec![1, 2, 4, 8, 16]
                } {
                    for checkpoint in 0..2 {
                        configs.push(Task {
                            job: Job {
                                scale,
                                dt,
                                case,
                                workers,
                                round: 1,
                            },
                            checkpoint: Some(checkpoint),
                        });
                    }
                }
            }
        }
    }
    if references || probe {
        return configs;
    }
    let mut result = Vec::new();
    for round in 1..=3 {
        let mut order = configs.clone();
        order.rotate_left(7 * (round as usize - 1));
        result.extend(order.into_iter().map(|mut t| {
            t.job.round = round;
            t
        }));
    }
    result
}

struct Running {
    task: Task,
    child: Child,
    start: Instant,
}
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn launch(
    binary: &Path,
    input: &Path,
    implementation: &str,
    output: &Path,
    probe: bool,
    task: Task,
) -> Result<Running, Error> {
    let log = File::create(output.join(format!("{}.log", task.key())))?;
    let mut command = Command::new(binary);
    command
        .arg(if task.checkpoint.is_none() {
            if probe {
                "restore-probe-reference"
            } else {
                "restore-reference"
            }
        } else {
            "restore-run"
        })
        .arg(input.join(format!("{}-a", task.job.identity())))
        .arg(implementation);
    if let Some(index) = task.checkpoint {
        command
            .arg(task.job.workers.to_string())
            .arg(output.join(task.reference()))
            .arg(index.to_string());
    }
    let child = command
        .arg(output.join(task.key()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log))
        .spawn()?;
    Ok(Running {
        task,
        child,
        start: Instant::now(),
    })
}

fn check_report(
    report: &Value,
    task: Task,
    implementation: &str,
    probe: bool,
) -> Result<(), Error> {
    let id = &report["input_manifest"]["identity"];
    let (warm, observation) = task.job.windows();
    let start = match task.job.case {
        "W1" | "W2" => {
            if task.checkpoint == Some(1) {
                if probe { warm + 128 } else { 2 * warm }
            } else {
                warm
            }
        }
        "W3" => warm + 1,
        _ => warm + u64::from(task.checkpoint == Some(1)),
    };
    let end = if probe {
        match task.job.case {
            "W4" => warm + 2,
            "W3" => warm + 257,
            _ => warm + 256,
        }
    } else {
        warm + observation
    };
    let valid = report["complete"] == true
        && report["implementation_commit"] == implementation
        && report["probe"] == probe
        && report["workers"] == task.job.workers
        && id["scale"] == task.job.scale
        && id["case"] == task.job.case
        && id["fixed_delta_ms"] == task.job.dt
        && report["end_tick"] == end;
    let valid = valid
        && if let Some(index) = task.checkpoint {
            report["format"] == "lf-synth-restore-run-1"
                && report["checkpoint"] == index
                && report["start_tick"] == start
                && report["after_commands"] == (task.job.case == "W3" && index == 1)
                && report["trace_records"] == end - start
                && report["attestation"] == "Pass"
                && [
                    "new_observation_stream",
                    "initial_state_and_mapping_equal",
                    "full_suffix_equal",
                    "final_state_equal",
                ]
                .iter()
                .all(|field| report[field] == true)
        } else {
            report["format"] == "lf-synth-restore-reference-1"
                && report["warm_up_ticks"] == warm
                && report["observation_ticks"] == observation
                && report["trace_records"] == end + 1
                && report["archives"].as_array().is_some_and(|a| a.len() == 2)
        };
    if !valid {
        return Err(Error::Invalid(format!(
            "恢复矩阵回执身份、阶段、窗口或证明不符：{}",
            task.key()
        )));
    }
    Ok(())
}

fn run_stage(
    binary: &Path,
    input: &Path,
    implementation: &str,
    output: &Path,
    probe: bool,
    jobs: Vec<Task>,
    summary: &mut Value,
) -> Result<(), Error> {
    run_stage_using(
        input,
        implementation,
        output,
        probe,
        jobs,
        summary,
        |task| launch(binary, input, implementation, output, probe, task),
    )
}

fn run_stage_using(
    input: &Path,
    implementation: &str,
    output: &Path,
    probe: bool,
    jobs: Vec<Task>,
    summary: &mut Value,
    mut launch_task: impl FnMut(Task) -> Result<Running, Error>,
) -> Result<(), Error> {
    let mut running = Vec::<Running>::new();
    let mut next = 0;
    let mut stopped = false;
    let mut completed = 0;
    while next < jobs.len() || !running.is_empty() {
        while !stopped
            && next < jobs.len()
            && running.len() < 4
            && running.iter().map(|r| r.task.job.workers).sum::<u32>() + jobs[next].job.workers
                <= 32
        {
            match launch_task(jobs[next]) {
                Ok(run) => {
                    summary["execution_order"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!({"run":run.task.key(),"pid":run.child.id()}));
                    running.push(run);
                    next += 1;
                }
                Err(e) => {
                    stopped = true;
                    summary["errors"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!({"launch":jobs[next].key(),"error":e.to_string()}));
                }
            }
            super::write(&output.join("summary.json"), summary)?;
        }
        for index in (0..running.len()).rev() {
            let Some(status) = running[index].child.try_wait()? else {
                continue;
            };
            let run = running.swap_remove(index);
            let result = (|| {
                if !status.success() {
                    return Err(Error::Invalid(format!(
                        "子进程失败：{} {status}",
                        run.task.key()
                    )));
                }
                let directory = output.join(run.task.key());
                let report: Value =
                    serde_json::from_slice(&fs::read(directory.join("report.json"))?)?;
                check_report(&report, run.task, implementation, probe)?;
                let manifest: Value = serde_json::from_slice(&fs::read(
                    input
                        .join(format!("{}-a", run.task.job.identity()))
                        .join("manifest.json"),
                )?)?;
                if report["input_manifest"] != manifest {
                    return Err(Error::Invalid("恢复矩阵输入 manifest 不符".into()));
                }
                let mut files = json!({});
                for name in [
                    "report.json",
                    "ticks.bin",
                    "events.bin",
                    "commands.bin",
                    "batches.json",
                    "final.json",
                ] {
                    files[name] = restore::file_identity(&directory.join(name))?;
                }
                if run.task.checkpoint.is_none() {
                    for i in 0..2 {
                        for suffix in ["json", "lfrs"] {
                            let name = format!("checkpoint-{i}.{suffix}");
                            files[&name] = restore::file_identity(&directory.join(&name))?;
                        }
                    }
                }
                summary["runs"].as_array_mut().unwrap().push(json!({"run":run.task.key(),"reference":run.task.reference(),"checkpoint":run.task.checkpoint,"round":run.task.job.round,"workers":run.task.job.workers,"wall_seconds":run.start.elapsed().as_secs_f64(),"report":report,"files":files}));
                Ok(())
            })();
            completed += 1;
            if let Err(error) = result {
                stopped = true;
                summary["errors"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"run":run.task.key(),"error":error.to_string()}));
            }
            super::write(&output.join("summary.json"), summary)?;
            eprintln!(
                "restore matrix stage {completed}/{} complete, stopped={stopped}",
                jobs.len()
            );
        }
        if stopped && running.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if stopped || completed != jobs.len() {
        return Err(Error::Invalid(
            "恢复矩阵首错停止；保留材料，不补跑或拼接".into(),
        ));
    }
    Ok(())
}

fn matrix(
    input: &Path,
    implementation: &str,
    environment: &Path,
    output: &Path,
    probe: bool,
) -> Result<(), Error> {
    fs::create_dir(output)?;
    let start = Instant::now();
    let mut summary = json!({"format":"lf-synth-restore-matrix-1","complete":false,"probe":probe,"implementation_commit":implementation,"expected_references":8,"expected_restores":if probe {32} else {240},"max_processes":4,"max_configured_workers":32,"execution_order":[],"runs":[],"errors":[]});
    super::write(&output.join("summary.json"), &summary)?;
    let result = (|| {
        let environment: Value = serde_json::from_slice(&fs::read(environment)?)?;
        if environment["implementation_commit"] != implementation
            || environment["source_clean"] != true
            || ["rustc_vv", "cargo_version", "target", "build_command"]
                .iter()
                .any(|f| !environment[f].as_str().is_some_and(|s| !s.is_empty()))
        {
            return Err(Error::Invalid("来源环境声明不完整".into()));
        }
        let binary = std::env::current_exe()?;
        let identity = restore::file_identity(&binary)?;
        summary["binary"] = identity.clone();
        summary["environment"] = environment;
        run_stage(
            &binary,
            input,
            implementation,
            output,
            probe,
            tasks(probe, true),
            &mut summary,
        )?;
        run_stage(
            &binary,
            input,
            implementation,
            output,
            probe,
            tasks(probe, false),
            &mut summary,
        )?;
        if identity != restore::file_identity(&binary)? {
            return Err(Error::Invalid("矩阵前后二进制变化".into()));
        }
        Ok(())
    })();
    summary["elapsed_seconds"] = json!(start.elapsed().as_secs_f64());
    match result {
        Ok(()) => summary["complete"] = json!(true),
        Err(ref e) => summary["error"] = json!(e.to_string()),
    };
    super::write(&output.join("summary.json"), &summary)?;
    result
}

pub(super) fn dispatch(args: &[OsString]) -> Option<Result<(), Error>> {
    let name = args.first()?.to_str()?;
    if !name.starts_with("restore-") {
        return None;
    }
    Some((|| {
        let implementation = args
            .get(2)
            .and_then(|v| v.to_str())
            .ok_or_else(|| Error::Invalid("恢复命令缺少实现提交".into()))?;
        match (name,args.len()) {
            ("restore-reference"|"restore-probe-reference",4)=>{restore::reference(Path::new(&args[1]),implementation,name=="restore-probe-reference",Path::new(&args[3]))?;},
            ("restore-run",7)=>{let workers:NonZeroU32=args[3].to_str().unwrap_or("").parse().map_err(|_|Error::Invalid("worker 必须是非零整数".into()))?;let index=args[5].to_str().unwrap_or("").parse().map_err(|_|Error::Invalid("检查点编号非法".into()))?;restore::resume(Path::new(&args[1]),implementation,ExecutionConfig::new(workers),Path::new(&args[4]),index,Path::new(&args[6]))?;},
            ("restore-matrix"|"restore-probe",5)=>matrix(Path::new(&args[1]),implementation,Path::new(&args[3]),Path::new(&args[4]),name=="restore-probe")?,
            _=>return Err(Error::Invalid("restore-reference INPUT SHA OUTPUT；restore-run INPUT SHA WORKERS REFERENCE CHECKPOINT OUTPUT；restore-matrix/restore-probe INPUT SHA ENV OUTPUT".into())),
        }
        Ok(())
    })())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn report_rejects_worker_phase_source_and_window_mismatch() {
        let task = Task {
            job: Job {
                scale: 10_000,
                case: "W3",
                dt: 16,
                workers: 16,
                round: 1,
            },
            checkpoint: Some(1),
        };
        let report = json!({"format":"lf-synth-restore-run-1","complete":true,"probe":false,"implementation_commit":"a".repeat(40),"input_manifest":{"identity":{"scale":10_000,"case":"W3","fixed_delta_ms":16}},"workers":16,"checkpoint":1,"start_tick":14_509,"after_commands":true,"end_tick":43_524,"trace_records":29_015,"attestation":"Pass","new_observation_stream":true,"initial_state_and_mapping_equal":true,"full_suffix_equal":true,"final_state_equal":true});
        check_report(&report, task, &"a".repeat(40), false).unwrap();
        for (field, value) in [
            ("workers", json!(1)),
            ("after_commands", json!(false)),
            ("implementation_commit", json!("b".repeat(40))),
            ("end_tick", json!(43_523)),
            ("trace_records", json!(29_014)),
            ("attestation", json!("Untrusted")),
            ("full_suffix_equal", json!(false)),
            ("complete", json!(false)),
            ("probe", json!(true)),
        ] {
            let mut bad = report.clone();
            bad[field] = value;
            assert!(
                check_report(&bad, task, &"a".repeat(40), false).is_err(),
                "{field}"
            );
        }
    }

    #[test]
    #[ignore = "由首错停止/子进程回收回归显式调用"]
    fn child_fixture() {
        let milliseconds: u64 = std::env::var("LF_RESTORE_CHILD_SLEEP_MS")
            .unwrap()
            .parse()
            .unwrap();
        std::thread::sleep(Duration::from_millis(milliseconds));
        if let Ok(path) = std::env::var("LF_RESTORE_CHILD_MARKER") {
            fs::write(path, b"finished").unwrap();
        }
    }

    fn child(task: Task, milliseconds: u64, marker: &Path) -> Running {
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "restore_matrix::tests::child_fixture",
                "--ignored",
            ])
            .env("LF_RESTORE_CHILD_SLEEP_MS", milliseconds.to_string())
            .env("LF_RESTORE_CHILD_MARKER", marker)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        Running {
            task,
            child,
            start: Instant::now(),
        }
    }

    #[test]
    fn first_error_stops_launching_and_collects_started_processes() {
        let directory = tempfile::tempdir().unwrap();
        let mut jobs = tasks(false, false).into_iter().take(3).collect::<Vec<_>>();
        for task in &mut jobs {
            task.job.workers = 16;
        }
        let mut summary = json!({"execution_order":[],"runs":[],"errors":[]});
        let mut launched = 0;
        let result = run_stage_using(
            directory.path(),
            &"a".repeat(40),
            directory.path(),
            false,
            jobs,
            &mut summary,
            |task| {
                launched += 1;
                Ok(child(
                    task,
                    if launched == 1 { 0 } else { 200 },
                    &directory.path().join(format!("done-{launched}")),
                ))
            },
        );
        assert!(result.is_err());
        assert_eq!(launched, 2);
        assert!(directory.path().join("done-1").exists());
        assert!(directory.path().join("done-2").exists());
        assert_eq!(summary["errors"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn drop_reaps_live_child_after_parent_error() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("done");
        let start = Instant::now();
        let run = child(tasks(false, false)[0], 10_000, &marker);
        drop(run);
        assert!(start.elapsed() < Duration::from_secs(5));
        assert!(!marker.exists());
    }
    #[test]
    fn matrix_has_exact_coverage_without_duplicate_jobs() {
        let jobs = tasks(false, false);
        assert_eq!(jobs.len(), 240);
        assert_eq!(
            jobs.iter()
                .map(|t| t.key())
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            240
        );
        assert_eq!(tasks(false, true).len(), 8);
        assert_eq!(tasks(true, false).len(), 32);
        for round in 1..=3 {
            for worker in [1, 2, 4, 8, 16] {
                assert_eq!(
                    jobs.iter()
                        .filter(|t| t.job.round == round && t.job.workers == worker)
                        .count(),
                    16
                );
            }
        }
    }
}
