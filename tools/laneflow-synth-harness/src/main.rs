use std::fs::{self, File};
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

use laneflow_synth_harness::{Error, run};
use serde_json::{Value, json};

fn write(path: &Path, value: &Value) -> Result<(), Error> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    fs::write(path, bytes)?;
    Ok(())
}

fn matrix(input: &Path, implementation: &str, output: &Path) -> Result<(), Error> {
    fs::create_dir(output)?;
    let binary = std::env::current_exe()?;
    let mut summary = json!({"format":"lf-synth-semantic-matrix-1","complete":false,
        "implementation_commit":implementation,"scope":"semantics_only_not_performance_or_product_certification","rows":[]});
    write(&output.join("summary.json"), &summary)?;
    let result = (|| {
        for scale in [10_000, 100_000] {
            for case in ["W1", "W2"] {
                for dt in [16, 33] {
                    let label = format!("{scale}-{case}-{dt}");
                    // 复用正式生成器矩阵的 -a 规范输入；两个消费进程均重新受检读取。
                    let bundle = input.join(format!("{label}-a"));
                    let mut reports = Vec::new();
                    for round in ["a", "b"] {
                        let target = output.join(format!("{label}-{round}"));
                        let log = File::create(output.join(format!("{label}-{round}.log")))?;
                        let status = Command::new(&binary)
                            .arg("run")
                            .arg(&bundle)
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
                        let report: Value =
                            serde_json::from_slice(&fs::read(target.join("report.json"))?)?;
                        if report["complete"] != true {
                            return Err(Error::Invalid(format!(
                                "不完整子进程回执 {label}-{round}"
                            )));
                        }
                        reports.push(report);
                    }
                    let a = fs::read(output.join(format!("{label}-a/ticks.bin")))?;
                    let b = fs::read(output.join(format!("{label}-b/ticks.bin")))?;
                    if a != b || reports[0] != reports[1] {
                        return Err(Error::Invalid(format!("独立进程轨迹或检查点不同：{label}")));
                    }
                    summary["rows"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!({"identity":label,
                        "fresh_process_rounds":2,"trace_bytes_equal":true,"reports_equal":true,
                        "trace_byte_length":a.len(),"report":reports.remove(0)}));
                    write(&output.join("summary.json"), &summary)?;
                    eprintln!(
                        "verified {label} ({}/8)",
                        summary["rows"].as_array().unwrap().len()
                    );
                }
            }
        }
        Ok::<_, Error>(())
    })();
    if let Err(error) = result {
        summary["error"] = Value::String(error.to_string());
        write(&output.join("summary.json"), &summary)?;
        return Err(error);
    }
    summary["complete"] = Value::Bool(true);
    write(&output.join("summary.json"), &summary)?;
    Ok(())
}

fn execute() -> Result<(), Error> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 1 && args[0] == "--help" {
        println!(
            "laneflow-synth-harness run <input-directory> <implementation-commit> <new-output-directory>\nlaneflow-synth-harness matrix <generator-matrix-root> <implementation-commit> <new-output-root>"
        );
        return Ok(());
    }
    if args.len() != 4 {
        return Err(Error::Invalid("参数错误；使用 --help 查看正式入口".into()));
    }
    let implementation = args[2]
        .to_str()
        .ok_or_else(|| Error::Invalid("提交必须为 UTF-8".into()))?;
    if args[0] == "run" {
        let result = run(Path::new(&args[1]), implementation, Path::new(&args[3]))?;
        println!("{}", serde_json::to_string(&result)?);
    } else if args[0] == "matrix" {
        matrix(Path::new(&args[1]), implementation, Path::new(&args[3]))?;
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
