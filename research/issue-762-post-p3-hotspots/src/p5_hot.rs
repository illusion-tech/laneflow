//! #808：直接热输入标量/SIMD 与正式 P5 的三臂整拍研究。
#[allow(dead_code)]
mod cache_research;
mod chunk_build;
mod chunk_collector;
mod chunk_config;
mod chunk_native;
mod environment;
mod hot_export;
mod simd_analysis;
#[allow(dead_code)]
mod simd_export;
use cache_research::{CapturePlan, Experiment, io};
use std::{error::Error, path::Path};
type Result<T> = std::result::Result<T, Box<dyn Error>>;
const BASE: &str = "fcd803f2cb28ba7a94f3f0ff589e7e9b7bdbd7d3";
const COLLECTOR_BIN: &str = "laneflow-p5-hot-simd-research";
const EXPERIMENT: Experiment = Experiment {
    baseline: BASE,
    protocol: "p5-hot-input-simd-v1",
    count_p2: false,
};
fn need(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(message.to_owned().into())
    }
}
fn plan() -> CapturePlan {
    CapturePlan {
        arms: &["base", "layout", "candidate"],
        scale: None,
        alternate_quartets: true,
        observe: environment::observe,
        bind_build: chunk_build::bind_capture,
        validate_builds: |identity| {
            chunk_build::validate_pair(identity)?;
            chunk_build::compatible(
                &identity["sources"]["base"]["build"],
                &identity["sources"]["layout"]["build"],
            )?;
            Ok(())
        },
    }
}
fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|s| s == "build-collector") && args.len() == 2 {
        return chunk_collector::build(Path::new(&args[1]));
    }
    if !args.first().is_some_and(|s| s == "native-toolchain") {
        let collector = chunk_collector::verify_running()?;
        if args
            .first()
            .is_some_and(|s| ["prepare", "run"].contains(&s.as_str()))
        {
            chunk_collector::verify_worktree(&collector)?;
        }
    }
    match args.first().map(String::as_str) {
        Some("verify-collector") if args.len() == 2 => chunk_collector::verify_root(Path::new(&args[1])),
        Some("native-toolchain") if args.len() == 2 => io::write_new(Path::new(&args[1]), &chunk_native::snapshot()?),
        Some("prepare") if args.len() == 3 => hot_export::export(Path::new(&args[2]), &args[1]),
        Some("run") if args.len() == 4 => cache_research::capture_planned_for("plain",
            Path::new(&args[1]), Path::new(&args[2]), Path::new(&args[3]), EXPERIMENT, plan()),
        Some("analyze" | "verify") if args.len() == 3 => {
            let raw = Path::new(&args[1]);
            let out = Path::new(&args[2]);
            let value = simd_analysis::analyze(raw)?;
            if args[0] == "verify" {
                need(value == io::read_json(out)?, "published mismatch")?;
                println!("verified 36 direct-hot-input runs");
                Ok(())
            } else { io::outside(raw, out)?; io::write_new(out, &value) }
        }
        _ => Err("build-collector <new-root> | verify-collector <root> | prepare <base|layout|candidate> <build-root> | run <build-root> <inputs> <new-raw> | analyze|verify <raw> <results>".into()),
    }
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
