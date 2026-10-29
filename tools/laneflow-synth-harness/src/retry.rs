use std::fs::{self, File};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

use laneflow_synth_harness::Error;
use serde_json::{Value, json};

use crate::write;

const TEST: &str = "kernel::lf_synth_contract::formal_retry::formal_input_failed_step_retry";

pub(crate) fn matrix(
    binary: &Path,
    input: &Path,
    implementation: &str,
    normal: &Path,
    output: &Path,
) -> Result<(), Error> {
    let normal_summary: Value = serde_json::from_slice(&fs::read(normal.join("summary.json"))?)?;
    ensure(
        normal_summary["complete"] == true
            && normal_summary["implementation_commit"] == implementation,
        "需要同一实现提交的完整正常矩阵",
    )?;
    fs::create_dir(output)?;
    let mut summary = json!({"format":"lf-synth-retry-matrix-1","complete":false,"implementation_commit":implementation,
        "max_concurrent_test_processes":2,"rows":[],"errors":[]});
    write(&output.join("summary.json"), &summary)?;
    let jobs: Vec<_> = [10_000, 100_000]
        .into_iter()
        .flat_map(|scale| {
            ["W3", "W4"].into_iter().flat_map(move |case| {
                [16, 33]
                    .into_iter()
                    .map(move |dt| format!("{scale}-{case}-{dt}"))
            })
        })
        .collect();
    let next = AtomicUsize::new(0);
    let (sender, receiver) = mpsc::channel();
    let result = std::thread::scope(|scope| {
        for _ in 0..2 {
            let (sender, next, jobs) = (sender.clone(), &next, &jobs);
            scope.spawn(move || {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(label) = jobs.get(index) else {
                        break;
                    };
                    let result = run_process(binary, input, implementation, normal, output, label);
                    if sender.send((label, result)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sender);
        for (label, result) in receiver {
            match result {
                Ok(report) => {
                    let rows = summary["rows"].as_array_mut().unwrap();
                    rows.push(json!({"identity":label,"normal_batch_and_state_match":true,"snapshot_bytes_equal":true,"report":report}));
                    rows.sort_by(|a, b| a["identity"].as_str().cmp(&b["identity"].as_str()));
                    eprintln!("verified retry {label} ({}/8)", rows.len());
                }
                Err(error) => summary["errors"]
                    .as_array_mut()
                    .unwrap()
                    .push(Value::String(format!("{label}: {error}"))),
            }
            write(&output.join("summary.json"), &summary)?;
        }
        ensure(
            summary["rows"].as_array().unwrap().len() == 8
                && summary["errors"].as_array().unwrap().is_empty(),
            "失败重试矩阵未全部通过",
        )
    });
    if let Err(error) = result {
        summary["error"] = Value::String(error.to_string());
        write(&output.join("summary.json"), &summary)?;
        return Err(error);
    }
    summary["complete"] = Value::Bool(true);
    write(&output.join("summary.json"), &summary)
}

fn run_process(
    binary: &Path,
    input: &Path,
    implementation: &str,
    normal: &Path,
    output: &Path,
    label: &str,
) -> Result<Value, Error> {
    let target = output.join(label);
    let log = File::create(output.join(format!("{label}.log")))?;
    let status = Command::new(binary)
        .args([
            TEST,
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("LF_SYNTH_INPUT", input.join(format!("{label}-a")))
        .env("LF_SYNTH_OUTPUT", &target)
        .env("LF_SYNTH_IMPLEMENTATION", implementation)
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .status()?;
    ensure(status.success(), &format!("test-only 子进程失败 {status}"))?;
    let report: Value = serde_json::from_slice(&fs::read(target.join("report.json"))?)?;
    let normal_report: Value =
        serde_json::from_slice(&fs::read(normal.join(format!("{label}-a/report.json")))?)?;
    verify_reports(&report, &normal_report, implementation, label)?;
    for (first, second) in [
        ("post-command.lfrs", "failed.lfrs"),
        ("post-command.lfrs", "fresh-post-command.lfrs"),
        ("retry.lfrs", "fresh.lfrs"),
    ] {
        ensure(
            fs::read(target.join(first))? == fs::read(target.join(second))?,
            &format!("规范快照文件不等 {first}/{second}"),
        )?;
    }
    Ok(report)
}

fn ensure(valid: bool, message: &str) -> Result<(), Error> {
    if valid {
        Ok(())
    } else {
        Err(Error::Invalid(message.into()))
    }
}

fn verify_reports(
    retry: &Value,
    normal: &Value,
    implementation: &str,
    label: &str,
) -> Result<(), Error> {
    let identity = &retry["input_manifest"]["identity"];
    let actual = format!(
        "{}-{}-{}",
        identity["scale"],
        identity["case"].as_str().unwrap_or(""),
        identity["fixed_delta_ms"]
    );
    ensure(
        retry["complete"] == true
            && normal["complete"] == true
            && retry["format"] == "lf-synth-retry-run-1"
            && normal["format"] == "lf-synth-burst-run-1"
            && actual == label
            && retry["implementation_commit"] == implementation
            && normal["implementation_commit"] == implementation
            && retry["input_manifest"] == normal["input_manifest"]
            && retry["step_input_digest"] == normal["step_input_digest"],
        "重试/正常输入身份或实现不符",
    )?;
    for flag in [
        "failed_snapshot_values_equal",
        "failed_events_equal",
        "failed_cursor_equal",
        "fresh_post_command_values_equal",
        "retry_fresh_values_equal",
        "retry_fresh_outcome_and_events_equal",
    ] {
        ensure(retry[flag] == true, &format!("缺少 guardrail 证明 {flag}"))?;
    }
    let boundary = retry["failure_boundary"]
        .as_u64()
        .ok_or_else(|| Error::Invalid("缺少失败边界".into()))?;
    let warm = normal["warm_up_ticks"]
        .as_u64()
        .ok_or_else(|| Error::Invalid("缺少预热长度".into()))?;
    ensure(
        boundary == warm + u64::from(identity["case"] == "W3")
            && retry["final_tick"] == warm + 2
            && retry["failpoint"] == "AfterTransitions"
            && retry["error"] == "ParkingObservationAllocFailed"
            && retry["failed_calls"] == 1,
        "失败位置、次数或错误类型不符",
    )?;
    let batch = normal["command_batches"]
        .as_array()
        .and_then(|batches| batches.iter().find(|batch| batch["boundary"] == boundary))
        .ok_or_else(|| Error::Invalid("正常运行缺少对应命令边界".into()))?;
    ensure(
        batch["input_digest"] == retry["command_batch_digest"]
            && batch["pre_command_state_digest"] == retry["pre_command_state_digest"]
            && batch["post_command_state_digest"] == retry["post_command_state_digest"]
            && batch["cursor_before"] == retry["cursor_before_commands"]
            && batch["cursor_after"] == retry["cursor_after_commands"]
            && retry["cursor_after_commands"] == retry["cursor_after_retry"],
        "正常与重试命令批次/状态/游标不同",
    )?;
    let checkpoint = |tick| {
        normal["checkpoints"]
            .as_array()
            .and_then(|points| points.iter().find(|p| p["tick"] == tick))
    };
    ensure(
        checkpoint(Value::from(boundary + 1))
            .is_some_and(|point| point["state_digest"] == retry["retry_state_digest"]),
        "重试状态与正常检查点不同",
    )?;
    ensure(
        checkpoint(retry["final_tick"].clone())
            .is_some_and(|point| point["state_digest"] == retry["final_state_digest"]),
        "最终状态与正常检查点不同",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_comparison_rejects_absent_or_mismatched_evidence() {
        let implementation = "a".repeat(40);
        assert!(verify_reports(&json!({}), &json!({}), &implementation, "10000-W4-33").is_err());
        let identity = json!({"scale":10000,"case":"W4","fixed_delta_ms":33});
        let mut retry = json!({"format":"lf-synth-retry-run-1","complete":true,"input_manifest":{"identity":identity},
            "implementation_commit":implementation,"step_input_digest":"step","failure_boundary":10,"final_tick":12,
            "command_batch_digest":"batch","pre_command_state_digest":"pre","post_command_state_digest":"post",
            "failpoint":"AfterTransitions","error":"ParkingObservationAllocFailed","failed_calls":1,
            "cursor_before_commands":1,"cursor_after_commands":3,"cursor_after_retry":3,"retry_state_digest":"retry","final_state_digest":"final"});
        for flag in [
            "failed_snapshot_values_equal",
            "failed_events_equal",
            "failed_cursor_equal",
            "fresh_post_command_values_equal",
            "retry_fresh_values_equal",
            "retry_fresh_outcome_and_events_equal",
        ] {
            retry[flag] = true.into();
        }
        let normal = json!({"format":"lf-synth-burst-run-1","complete":true,"input_manifest":retry["input_manifest"],"implementation_commit":implementation,"step_input_digest":"step","warm_up_ticks":10,
            "command_batches":[{"boundary":10,"input_digest":"batch","pre_command_state_digest":"pre","post_command_state_digest":"post","cursor_before":1,"cursor_after":3}],
            "checkpoints":[{"tick":11,"state_digest":"retry"},{"tick":12,"state_digest":"final"}]});
        verify_reports(&retry, &normal, &implementation, "10000-W4-33").unwrap();
        for (field, replacement) in [
            ("command_batch_digest", json!("other")),
            ("post_command_state_digest", json!("other")),
            ("cursor_after_retry", json!(4)),
            ("retry_state_digest", json!("other")),
        ] {
            let mut changed = retry.clone();
            changed[field] = replacement;
            assert!(verify_reports(&changed, &normal, &implementation, "10000-W4-33").is_err());
        }
    }
}
