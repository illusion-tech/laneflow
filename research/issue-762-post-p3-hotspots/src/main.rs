//! #762 独立研究工具：导出、采集、统计与封存验证，不进入交通运行时。
mod analysis;
mod capture;
mod io;
mod prepare;

use ::std::error::Error;
use serde_json::Value;
use std::path::Path;

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const BASE: &str = "37c5e1af3c72b0f60cf2dd889bbf6713b2a77054";
const ORDER: [&str; 6] = ["plain", "stages", "stages", "plain", "plain", "stages"];
const STAGES: [&str; 18] = [
    "Preflight",
    "Occupancy",
    "WaitingPrepare",
    "ConflictPrepare",
    "MotionLoop",
    "WaitingFinalize",
    "Signals",
    "ConflictFinalize",
    "WaitingOutputs",
    "Commit",
    "Frontier",
    "P4",
    "P3Discover",
    "P3Dispatch",
    "P3Consume",
    "P3Fused",
    "P3Tail",
    "Workload",
];

fn need(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(message.to_owned().into())
    }
}

fn string(value: &Value) -> Result<&str> {
    value.as_str().ok_or_else(|| "expected string".into())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = "prepare <plain|stages|detail> <new-source> <new-index> | run <base|detail> <new-output> <inputs> [parent-results] | analyze|verify <base|detail> <raw> <results> [parent-results]";
    need(args.len() >= 4, usage)?;
    let repo = std::env::current_dir()?;
    let mode = args[1].as_str();
    match args[0].as_str() {
        "prepare" if args.len() == 4 => {
            need(["plain", "stages", "detail"].contains(&mode), usage)?;
            io::ensure_new(Path::new(&args[3]))?;
            let index = prepare::export(&repo, Path::new(&args[2]), mode)?;
            io::write_new(Path::new(&args[3]), &index)?;
        }
        "run" => {
            need(
                (mode == "base" && args.len() == 4) || (mode == "detail" && args.len() == 5),
                usage,
            )?;
            capture::acquire(
                &repo,
                mode,
                Path::new(&args[2]),
                Path::new(&args[3]),
                args.get(4).map(Path::new),
            )?;
        }
        "analyze" | "verify" => {
            need(
                (mode == "base" && args.len() == 4) || (mode == "detail" && args.len() == 5),
                usage,
            )?;
            let raw = Path::new(&args[2]);
            let output = Path::new(&args[3]);
            let result = analysis::analyze(raw, args.get(4).map(Path::new))?;
            if args[0] == "verify" {
                analysis::same_json(&result, &io::read_json(output)?, "published")?;
                println!(
                    "verified {mode}: {} runs",
                    result["runs"].as_array().ok_or("runs")?.len()
                );
            } else {
                io::outside(raw, output)?;
                io::write_new(output, &result)?;
            }
        }
        _ => return Err(usage.into()),
    }
    Ok(())
}
