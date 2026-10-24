use std::{
    fs,
    path::Path,
    process::Stdio,
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use super::{
    Config, MAX_SECONDS,
    control::{self, Status},
    run, supervisor,
};

fn read(path: &Path) -> Value {
    control::read_json(path).unwrap()
}

fn wait_status(output: &Path, state: &str) -> Status {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(status) = control::read_json::<Status>(&output.join("status.json"))
            && status.state == state
        {
            return status;
        }
        assert!(
            !output.join("failure.json").exists(),
            "unexpected failure: {:?}",
            fs::read_to_string(output.join("failure.json"))
        );
        assert!(Instant::now() < deadline, "state timeout: {state}");
        thread::sleep(Duration::from_millis(20));
    }
}

fn config(seconds: u64) -> Config {
    let mut config = Config::new(seconds);
    config.batch_ticks = 4;
    config.wall_seconds = 30;
    config
}

fn spawn(output: &Path, config: Config) -> thread::JoinHandle<crate::Result<()>> {
    let output = output.to_owned();
    thread::spawn(move || supervisor::supervise(&output, config, "integrity_soak::resident_worker"))
}

fn wait_first_segment(output: &Path) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let path = output.join("segments/000000/request.result.json");
        if let Ok(value) = control::read_json::<Value>(&path)
            && value["classification"] == "CompleteMatch"
        {
            return;
        }
        assert!(Instant::now() < deadline, "segment timeout");
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn maximum_long_window_builds_and_replays_a_short_chain() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("maximum-plan");
    // 只验证最大计划与短链路，不实际运行七天或把它计入长期样本。
    run(&output, MAX_SECONDS, false).unwrap();
    let summary = read(&output.join("summary.json"));
    assert_eq!(summary["scope"], "short-soak-chain-validation");
    assert_eq!(summary["complete_match_batches"], 1);
}

#[test]
fn rejected_output_and_duration_preserve_existing_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("existing");
    fs::create_dir(&output).unwrap();
    let failure = output.join("failure.json");
    fs::write(&failure, b"previous run failure evidence").unwrap();
    for seconds in [0, 1, MAX_SECONDS + 1] {
        assert!(run(&output, seconds, false).is_err());
        assert_eq!(
            fs::read(&failure).unwrap(),
            b"previous run failure evidence"
        );
        assert_eq!(fs::read_dir(&output).unwrap().count(), 1);
    }
    let mut command = supervisor::command("integrity_soak::bounded_soak").unwrap();
    let status = command
        .env("LF_SOAK_OUTPUT", &output)
        .env("LF_SOAK_SECONDS", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(!status.success());
    assert_eq!(
        fs::read(&failure).unwrap(),
        b"previous run failure evidence"
    );
    for seconds in [0, MAX_SECONDS + 1] {
        let fresh = temp.path().join(format!("invalid-{seconds}"));
        assert!(run(&fresh, seconds, false).is_err());
        assert!(!fresh.exists());
    }
    let long = Config::new(7_200);
    long.validate().unwrap();
    for invalid in [
        Config {
            batch_ticks: 0,
            ..long.clone()
        },
        Config {
            replay_seconds: 0,
            ..long.clone()
        },
        Config {
            max_disk_bytes: 0,
            ..long.clone()
        },
        Config {
            max_memory_bytes: 0,
            ..long.clone()
        },
        Config {
            wall_seconds: 1,
            ..long.clone()
        },
    ] {
        assert!(invalid.validate().is_err());
    }
}

#[test]
fn safe_pause_resumes_the_resident_world_and_excludes_paused_time() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("pause");
    let running = spawn(&output, config(4));
    let before = wait_status(&output, "Running");
    wait_first_segment(&output);
    control::issue(&output, "pause").unwrap();
    let paused = wait_status(&output, "Paused");
    assert_eq!(
        (paused.resident_pid, paused.world_id),
        (before.resident_pid, before.world_id)
    );
    let segment_count = fs::read_dir(output.join("segments")).unwrap().count();
    thread::sleep(Duration::from_millis(300));
    let still: Status = control::read_json(&output.join("status.json")).unwrap();
    assert_eq!(
        (still.tick, still.command_cursor, still.active_seconds),
        (paused.tick, paused.command_cursor, paused.active_seconds)
    );
    assert_eq!(
        fs::read_dir(output.join("segments")).unwrap().count(),
        segment_count
    );
    assert!(control::issue(&output, "pause").is_err());
    control::issue(&output, "resume").unwrap();
    wait_status(&output, "Running");
    control::issue(&output, "pause").unwrap();
    let second = wait_status(&output, "Paused");
    assert_eq!(
        (second.world_id, second.resident_pid),
        (before.world_id, before.resident_pid)
    );
    assert!(second.tick >= paused.tick);
    thread::sleep(Duration::from_millis(100));
    control::issue(&output, "resume").unwrap();
    running.join().unwrap().unwrap();
    let summary = read(&output.join("summary.json"));
    assert_eq!(summary["status"], "Completed");
    assert_eq!(summary["resident_pid"], before.resident_pid);
    assert_eq!(summary["world_id"], before.world_id);
    assert!(summary["paused_seconds"].as_f64().unwrap() >= 0.4);
    assert!(summary["active_seconds"].as_f64().unwrap() >= 4.0);
    assert!(summary["final_tick"].as_u64().unwrap() > paused.tick);
    assert_eq!(
        summary["covered_ticks"].as_u64().unwrap(),
        summary["final_tick"].as_u64().unwrap() - summary["initial_tick"].as_u64().unwrap()
    );
    let resumed = read(&output.join("events/00000002-resumed.json"));
    assert_eq!(resumed["tick"], paused.tick);
    assert!(!resumed["root_attestation"].as_str().unwrap().is_empty());
    assert_eq!(
        read(&output.join("events/00000004-resumed.json"))["tick"],
        second.tick
    );
    assert_eq!(
        read(&output.join("supervisor.result.json"))["status"],
        "Completed"
    );
    assert!(control::issue(&output, "resume").is_err());
}

#[test]
fn resume_rejects_changed_checkpoint_and_source_without_advancing() {
    for corrupt_source in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("changed");
        let running = spawn(&output, config(5));
        wait_status(&output, "Running");
        control::issue(&output, "pause").unwrap();
        let paused = wait_status(&output, "Paused");
        if corrupt_source {
            fs::write(output.join("source/network.lfca"), b"truncated source").unwrap();
        } else {
            let path = output.join("events/00000001-checkpoint.json");
            let mut checkpoint = read(&path);
            checkpoint["sample"]["tick"] = json!(paused.tick + 1);
            control::publish(&path, &checkpoint).unwrap();
        }
        control::issue(&output, "resume").unwrap();
        assert!(running.join().unwrap().is_err());
        let after: Status = control::read_json(&output.join("status.json")).unwrap();
        assert_eq!((after.tick, after.state.as_str()), (paused.tick, "Paused"));
        assert!(!output.join("events/00000002-resumed.json").exists());
        assert!(!output.join("summary.json").exists());
        assert_eq!(
            read(&output.join("supervisor.result.json"))["status"],
            "Incomplete"
        );
    }
}

#[test]
fn stop_while_paused_preserves_complete_segments_and_incomplete_collection() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("stop");
    let running = spawn(&output, config(10));
    wait_status(&output, "Running");
    wait_first_segment(&output);
    control::issue(&output, "pause").unwrap();
    wait_status(&output, "Paused");
    let records: Vec<_> = fs::read_dir(output.join("segments"))
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path().join("request.result.json");
            (path.clone(), fs::read(path).unwrap())
        })
        .collect();
    control::issue(&output, "stop").unwrap();
    assert!(running.join().unwrap().is_err());
    assert_eq!(read(&output.join("summary.json"))["status"], "Incomplete");
    for (path, bytes) in records {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    assert!(control::issue(&output, "resume").is_err());
}

#[test]
fn total_wall_budget_includes_paused_time() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("wall");
    let mut config = config(5);
    config.wall_seconds = 5;
    let running = spawn(&output, config);
    wait_status(&output, "Running");
    control::issue(&output, "pause").unwrap();
    wait_status(&output, "Paused");
    assert!(running.join().unwrap().is_err());
    let result = read(&output.join("supervisor.result.json"));
    assert!(result["reason"].as_str().unwrap().contains("wall timeout"));
    assert_eq!(result["cleanup"], "observed descendants terminated");
}

#[test]
fn sampled_memory_and_evidence_thresholds_stop_and_preserve_incomplete() {
    for memory in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("limit");
        let mut config = config(5);
        if memory {
            config.max_memory_bytes = 1;
        } else {
            config.max_disk_bytes = 1;
        }
        assert!(supervisor::supervise(&output, config, "integrity_soak::resident_worker").is_err());
        let result = read(&output.join("supervisor.result.json"));
        assert_eq!(result["status"], "Incomplete");
        assert!(result["reason"].as_str().unwrap().contains(if memory {
            "memory threshold"
        } else {
            "evidence threshold"
        }));
        assert_eq!(result["cleanup"], "observed descendants terminated");
        assert_eq!(
            read(&output.join("collection.json"))["status"],
            "Incomplete"
        );
    }
}

#[test]
fn replay_timeout_does_not_upgrade_pending_segment() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("request.json");
    let pending = path.with_extension("result.json");
    fs::write(&pending, b"Incomplete: audit pending").unwrap();
    assert!(
        super::replay_entry(
            &path,
            Duration::from_millis(50),
            "integrity_soak::tests::hanging_child"
        )
        .is_err()
    );
    assert_eq!(fs::read(pending).unwrap(), b"Incomplete: audit pending");
    assert!(!path.with_extension("response.json").exists());
}

#[test]
fn supervisor_cleans_observed_grandchild_on_timeout() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("tree");
    let mut config = config(2);
    config.wall_seconds = 2;
    assert!(
        supervisor::supervise(&output, config, "integrity_soak::tests::hanging_parent").is_err()
    );
    let tree = read(&output.join("fixture-tree.json"));
    let result = read(&output.join("supervisor.result.json"));
    assert_eq!(result["cleanup"], "observed descendants terminated");
    let mut system = sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    for key in ["parent_pid", "child_pid"] {
        let process = system.process(sysinfo::Pid::from_u32(tree[key].as_u64().unwrap() as u32));
        assert!(
            process.is_none_or(|process| process.status() == sysinfo::ProcessStatus::Zombie),
            "live descendant: {key}"
        );
    }
}

#[test]
fn premature_success_is_incomplete_at_the_supervisor() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("premature");
    assert!(
        supervisor::supervise(
            &output,
            config(5),
            "integrity_soak::tests::premature_success"
        )
        .is_err()
    );
    assert_eq!(read(&output.join("summary.json"))["status"], "Completed");
    assert_eq!(
        read(&output.join("supervisor.result.json"))["status"],
        "Incomplete"
    );
}

#[test]
fn supervisor_pipe_disconnect_stops_a_paused_resident() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("pipe");
    supervisor::create_output(&output, config(5)).unwrap();
    let mut command = supervisor::command("integrity_soak::resident_worker").unwrap();
    let child = command
        .env("LF_SOAK_OUTPUT", &output)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut guard = supervisor::ChildGuard::new(child);
    wait_status(&output, "Running");
    control::issue(&output, "pause").unwrap();
    wait_status(&output, "Paused");
    drop(guard.child.stdin.take());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = guard.child.try_wait().unwrap() {
            assert!(!status.success());
            break;
        }
        assert!(Instant::now() < deadline, "resident orphaned");
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        read(&output.join("failure.json"))["reason"]
            .as_str()
            .unwrap()
            .contains("supervisor disconnected")
    );
}

#[test]
#[ignore = "资源看护超时回归夹具，只由指定子进程调用"]
fn hanging_child() {
    thread::sleep(Duration::from_secs(60));
}

#[test]
#[ignore = "资源看护后代清理回归夹具，只由指定子进程调用"]
fn hanging_parent() {
    let mut child = supervisor::command("integrity_soak::tests::hanging_child")
        .unwrap()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let output = std::path::PathBuf::from(std::env::var_os("LF_SOAK_OUTPUT").unwrap());
    control::publish(
        &output.join("fixture-tree.json"),
        &json!({
            "parent_pid":std::process::id(), "child_pid":child.id(),
        }),
    )
    .unwrap();
    thread::sleep(Duration::from_secs(60));
    let _ = child.wait();
}

#[test]
#[ignore = "短运行不得伪装完整采集的负向回归夹具"]
fn premature_success() {
    let output = std::path::PathBuf::from(std::env::var_os("LF_SOAK_OUTPUT").unwrap());
    let spec: supervisor::Spec = control::read_json(&output.join("config.json")).unwrap();
    control::publish(&output.join("summary.json"), &json!({
        "status":"Completed", "run_id":spec.run_id, "scope":"bounded-resident-soak", "paced":true,
        "resident_pid":std::process::id(), "requested_seconds":spec.config.seconds,
        "runtime_workers":1, "world_id":1, "complete_match_batches":1,
        "active_seconds":spec.config.seconds, "paused_seconds":0, "initial_tick":1, "final_tick":2, "covered_ticks":1,
    })).unwrap();
}
