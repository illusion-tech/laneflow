use laneflow_urban_harness::{Artifacts, ResolvedPlan, Window, compare_runs, run_to_directory};
use std::path::Path;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("plan") if args.len() == 3 || (args.len() == 5 && args[3] == "--probe-ticks") => {
            let artifacts = Artifacts::load(Path::new(&args[1]))?;
            let window = if args.len() == 5 {Window::probe(args[4].parse()?)?} else {Window::correctness(&artifacts)?};
            let plan = ResolvedPlan::mixed(&artifacts, window)?;
            println!("{}", plan.write(Path::new(&args[2]))?);
        }
        Some("run") if args.len() == 4 => {
            let artifacts = Artifacts::load(Path::new(&args[1]))?;
            let plan = ResolvedPlan::read(Path::new(&args[2]))?;
            let result = run_to_directory(&artifacts, &plan, Path::new(&args[3]))?;
            println!("{}: {}/{} ticks", result.status, result.completed_ticks, result.expected_ticks);
            if let Some(error) = result.error {return Err(error.into());}
        }
        Some("compare") if args.len() == 4 => {
            let report = compare_runs(Path::new(&args[1]), Path::new(&args[2]))?;
            report.write(Path::new(&args[3]))?;
            println!("{}/{}: {}; report={}", report.case, report.scale, report.status, args[3]);
        }
        _ => return Err("usage: laneflow-urban-harness plan <artifacts> <plan.toml> [--probe-ticks N] | run <artifacts> <plan.toml> <new-output> | compare <run-a> <run-b> <new-comparison.json>".into()),
    }
    Ok(())
}
