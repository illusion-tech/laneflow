use std::fs::{self, File};
use std::num::NonZeroU32;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

use laneflow_runtime::ExecutionConfig;
use laneflow_synth_harness::{Error, run};
use serde_json::{Value, json};

mod retry;
mod workers;

fn write(path: &Path, value: &Value) -> Result<(), Error> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    fs::write(path, bytes)?;
    Ok(())
}

fn matrix(
    input: &Path,
    implementation: &str,
    output: &Path,
    cases: [&str; 2],
) -> Result<(), Error> {
    fs::create_dir(output)?;
    let binary = std::env::current_exe()?;
    let mut summary = json!({"format":"lf-synth-semantic-matrix-1","complete":false,
        "implementation_commit":implementation,"max_concurrent_processes":4,
        "scope":"semantics_only_not_performance_or_product_certification","rows":[],"errors":[]});
    write(&output.join("summary.json"), &summary)?;
    let mut jobs = Vec::new();
    for scale in [10_000, 100_000] {
        for case in cases {
            for dt in [16, 33] {
                for round in ["a", "b"] {
                    jobs.push((format!("{scale}-{case}-{dt}"), round));
                }
            }
        }
    }
    let next = AtomicUsize::new(0);
    let (sender, receiver) = mpsc::channel();
    let mut reports = vec![None; jobs.len()];
    let result = std::thread::scope(|scope| {
        for _ in 0..4 {
            let (sender, next, jobs, binary) = (sender.clone(), &next, &jobs, &binary);
            scope.spawn(move || {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some((label, round)) = jobs.get(index) else {
                        break;
                    };
                    let result = run_process(binary, input, implementation, output, label, round);
                    if sender.send((index, result)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sender);
        // 只有父进程写汇总；scope 等待已启动子进程，错误也不遗留孤儿进程。
        for (index, result) in receiver {
            let paired = index ^ 1;
            let result = result.and_then(|report| {
                reports[index] = Some(report);
                if let (Some(a), Some(b)) = (&reports[index], &reports[paired]) {
                    let label = &jobs[index].0;
                    let row = compare(output, label, a, b)?;
                    let rows = summary["rows"].as_array_mut().unwrap();
                    rows.push(row);
                    rows.sort_by(|a, b| a["identity"].as_str().cmp(&b["identity"].as_str()));
                    eprintln!("verified {label} ({}/8)", rows.len());
                }
                Ok(())
            });
            if let Err(error) = result {
                summary["errors"]
                    .as_array_mut()
                    .unwrap()
                    .push(Value::String(error.to_string()));
            }
            write(&output.join("summary.json"), &summary)?;
        }
        if summary["rows"].as_array().unwrap().len() != 8
            || !summary["errors"].as_array().unwrap().is_empty()
        {
            return Err(Error::Invalid(
                "矩阵未全部通过；具体失败见 summary.json".into(),
            ));
        }
        Ok(())
    });
    if let Err(error) = result {
        summary["error"] = Value::String(error.to_string());
        write(&output.join("summary.json"), &summary)?;
        return Err(error);
    }
    summary["complete"] = Value::Bool(true);
    write(&output.join("summary.json"), &summary)?;
    Ok(())
}

fn run_process(
    binary: &Path,
    input: &Path,
    implementation: &str,
    output: &Path,
    label: &str,
    round: &str,
) -> Result<Value, Error> {
    let target = output.join(format!("{label}-{round}"));
    let log = File::create(output.join(format!("{label}-{round}.log")))?;
    let status = Command::new(binary)
        .arg("run")
        .arg(input.join(format!("{label}-a")))
        .arg(implementation)
        .arg(&target)
        .stdout(Stdio::null())
        .stderr(Stdio::from(log))
        .status()?;
    if !status.success() {
        return Err(Error::Invalid(format!(
            "子进程失败 {label}-{round}: {status}"
        )));
    }
    let report: Value = serde_json::from_slice(&fs::read(target.join("report.json"))?)?;
    let identity = &report["input_manifest"]["identity"];
    let actual = format!(
        "{}-{}-{}",
        identity["scale"],
        identity["case"].as_str().unwrap_or(""),
        identity["fixed_delta_ms"]
    );
    if report["complete"] != true || actual != label {
        return Err(Error::Invalid(format!(
            "不完整或身份不符的子进程回执 {label}-{round}: {actual}"
        )));
    }
    Ok(report)
}

fn compare(output: &Path, label: &str, a: &Value, b: &Value) -> Result<Value, Error> {
    let first = fs::read(output.join(format!("{label}-a/ticks.bin")))?;
    let second = fs::read(output.join(format!("{label}-b/ticks.bin")))?;
    if first != second || a != b {
        return Err(Error::Invalid(format!("独立进程轨迹或检查点不同：{label}")));
    }
    if a["format"] == "lf-synth-burst-run-1" {
        for file in ["commands.bin", "batches.json"] {
            let first = fs::read(output.join(format!("{label}-a/{file}")))?;
            let second = fs::read(output.join(format!("{label}-b/{file}")))?;
            if first != second {
                return Err(Error::Invalid(format!(
                    "独立进程命令证据不同：{label}/{file}"
                )));
            }
        }
    }
    Ok(
        json!({"identity":label,"fresh_process_rounds":2,"trace_bytes_equal":true,"reports_equal":true,
        "trace_byte_length":first.len(),"report":a}),
    )
}

fn execute() -> Result<(), Error> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 1 && args[0] == "--help" {
        println!(
            "laneflow-synth-harness run <input-directory> <implementation-commit> <new-output-directory>\nlaneflow-synth-harness matrix <generator-matrix-root> <implementation-commit> <new-output-root>\nlaneflow-synth-harness burst-matrix <generator-matrix-root> <implementation-commit> <new-output-root>\nlaneflow-synth-harness retry-matrix <runtime-test-binary> <generator-matrix-root> <implementation-commit> <normal-matrix-output> <new-output-root>\nlaneflow-synth-harness worker-run <input-directory> <implementation-commit> <workers> <new-output-directory>\nlaneflow-synth-harness worker-matrix <generator-matrix-root> <implementation-commit> <environment.json> <new-output-root>"
        );
        return Ok(());
    }
    if args.len() == 6 && args[0] == "retry-matrix" {
        let implementation = args[3]
            .to_str()
            .ok_or_else(|| Error::Invalid("提交必须为 UTF-8".into()))?;
        return retry::matrix(
            Path::new(&args[1]),
            Path::new(&args[2]),
            implementation,
            Path::new(&args[4]),
            Path::new(&args[5]),
        );
    }
    if args.len() == 5 && (args[0] == "worker-run" || args[0] == "worker-matrix") {
        let implementation = args[2]
            .to_str()
            .ok_or_else(|| Error::Invalid("提交必须为 UTF-8".into()))?;
        if args[0] == "worker-matrix" {
            return workers::matrix(
                Path::new(&args[1]),
                implementation,
                Path::new(&args[3]),
                Path::new(&args[4]),
            );
        }
        let count: NonZeroU32 = args[3]
            .to_str()
            .ok_or_else(|| Error::Invalid("worker 数必须为 UTF-8".into()))?
            .parse()
            .map_err(|_| Error::Invalid("worker 数必须是非零 u32".into()))?;
        let report = run(
            Path::new(&args[1]),
            implementation,
            ExecutionConfig::new(count),
            Path::new(&args[4]),
        )?;
        println!("{}", serde_json::to_string(&report)?);
        return Ok(());
    }
    if args.len() != 4 {
        return Err(Error::Invalid("参数错误；使用 --help 查看正式入口".into()));
    }
    let implementation = args[2]
        .to_str()
        .ok_or_else(|| Error::Invalid("提交必须为 UTF-8".into()))?;
    if args[0] == "run" {
        let result = run(
            Path::new(&args[1]),
            implementation,
            ExecutionConfig::new(NonZeroU32::MIN),
            Path::new(&args[3]),
        )?;
        println!("{}", serde_json::to_string(&result)?);
    } else if args[0] == "matrix" || args[0] == "burst-matrix" {
        let cases = if args[0] == "matrix" {
            ["W1", "W2"]
        } else {
            ["W3", "W4"]
        };
        matrix(
            Path::new(&args[1]),
            implementation,
            Path::new(&args[3]),
            cases,
        )?;
    } else {
        return Err(Error::Invalid("未知子命令".into()));
    }
    Ok(())
}

fn main() -> ExitCode {
    match execute() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn comparison_rejects_trace_or_checkpoint_changes() {
        let root = tempfile::tempdir().unwrap();
        for round in ["a", "b"] {
            let path = root.path().join(format!("case-{round}"));
            std::fs::create_dir(&path).unwrap();
            std::fs::write(path.join("ticks.bin"), [1_u8, 2, 3]).unwrap();
        }
        let report = serde_json::json!({"checkpoint":"original"});
        assert!(super::compare(root.path(), "case", &report, &report).is_ok());
        let changed = serde_json::json!({"checkpoint":"changed"});
        assert!(super::compare(root.path(), "case", &report, &changed).is_err());
        std::fs::write(root.path().join("case-b/ticks.bin"), [1_u8, 9, 3]).unwrap();
        assert!(super::compare(root.path(), "case", &report, &report).is_err());
        std::fs::write(root.path().join("case-b/ticks.bin"), [1_u8, 2, 3]).unwrap();
        let burst = serde_json::json!({"format":"lf-synth-burst-run-1"});
        for round in ["a", "b"] {
            for file in ["commands.bin", "batches.json"] {
                std::fs::write(root.path().join(format!("case-{round}/{file}")), [1_u8, 2])
                    .unwrap();
            }
        }
        assert!(super::compare(root.path(), "case", &burst, &burst).is_ok());
        std::fs::write(root.path().join("case-b/commands.bin"), [2_u8, 1]).unwrap();
        assert!(super::compare(root.path(), "case", &burst, &burst).is_err());
    }
}
