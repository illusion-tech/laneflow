//! #801 P5 分块粒度：普通整拍平衡 A/B 与独立的块级经过时间诊断。
#[allow(dead_code)]
mod cache_research;
mod chunk_analysis;
mod chunk_build;
mod chunk_export;
mod chunk_native;
mod environment;
use cache_research::{CapturePlan, Experiment, io};
use std::{error::Error, path::Path};
type Result<T> = std::result::Result<T, Box<dyn Error>>;
const BASE: &str = "cb562bde948b6f96484581b650422f149f6c5787";
const EXPERIMENT: Experiment = Experiment {
    baseline: BASE,
    protocol: "p5-chunk-grain-v3",
    count_p2: false,
};
const STAGES: [&str; 12] = [
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
];
fn need(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(message.to_owned().into())
    }
}
fn plan(mode: &str) -> Result<CapturePlan> {
    need(["plain", "detail"].contains(&mode), "mode")?;
    Ok(CapturePlan {
        scale: if mode == "detail" { Some("100k") } else { None },
        alternate_quartets: true,
        observe: environment::observe,
        bind_build: chunk_build::bind_capture,
        validate_builds: chunk_build::validate_pair,
    })
}
fn run() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    match a.first().map(String::as_str) {
        Some("native-toolchain") if a.len() == 2 => io::write_new(Path::new(&a[1]), &chunk_native::snapshot()?),
        Some("prepare") if a.len() == 4 => chunk_export::export(Path::new(&a[3]), &a[1], &a[2]),
        Some("run") if a.len() == 5 => cache_research::capture_planned_for(&a[1], Path::new(&a[2]), Path::new(&a[3]), Path::new(&a[4]), EXPERIMENT, plan(&a[1])?),
        Some("analyze" | "verify") if a.len() == 3 || a.len() == 4 => {
            let raw = Path::new(&a[1]);
            let out = Path::new(&a[2]);
            let value = chunk_analysis::analyze(raw, a.get(3).map(Path::new))?;
            if a[0] == "verify" {
                need(value == io::read_json(out)?, "published mismatch")?;
                println!("verified {} runs", value["runs"].as_array().ok_or("runs")?.len());
                Ok(())
            } else { io::outside(raw, out)?; io::write_new(out, &value) }
        }
        _ => Err("prepare <base|candidate> <plain|detail> <new-build-root> | run <plain|detail> <root> <inputs> <new-raw> | analyze|verify <raw> <results> [plain-raw required for detail]".into()),
    }
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
