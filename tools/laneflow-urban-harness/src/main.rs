use laneflow_urban_harness::{
    Artifacts, ResolvedPlan, UrbanCase, Window, compare_performance_runs, compare_runs,
    run_to_directory,
};
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
        Some("plan") if args.len() >= 3 => {
            let artifacts = Artifacts::load(Path::new(&args[1]))?;
            let mut case = UrbanCase::MixedPeak;
            let mut probe_ticks = None;
            let mut probe_warm_up = 0;
            let mut performance = false;
            let mut index = 3;
            while index < args.len() {
                match args[index].as_str() {
                    "--case" if index + 1 < args.len() => {
                        case = args[index + 1].parse()?;
                        index += 2;
                    }
                    "--probe-ticks" if index + 1 < args.len() => {
                        probe_ticks = Some(args[index + 1].parse()?);
                        index += 2;
                    }
                    "--probe-warm-up" if index + 1 < args.len() => {
                        probe_warm_up = args[index + 1].parse()?;
                        index += 2;
                    }
                    "--performance" => {
                        performance = true;
                        index += 1;
                    }
                    _ => return Err(usage().into()),
                }
            }
            let window = if performance && (probe_ticks.is_some() || probe_warm_up != 0) {
                return Err("--performance cannot be combined with probe options".into());
            } else if performance {
                Window::performance(&artifacts)?
            } else if let Some(ticks) = probe_ticks {
                Window::probe_after(probe_warm_up, ticks)?
            } else if probe_warm_up != 0 {
                return Err("--probe-warm-up requires --probe-ticks".into());
            } else {
                Window::correctness(&artifacts)?
            };
            let plan = ResolvedPlan::for_case(&artifacts, case, window)?;
            println!("{}", plan.write(Path::new(&args[2]))?);
        }
        Some("run") if args.len() == 4 => {
            let artifacts = Artifacts::load(Path::new(&args[1]))?;
            let plan = ResolvedPlan::read(Path::new(&args[2]))?;
            let result = run_to_directory(&artifacts, &plan, Path::new(&args[3]))?;
            println!(
                "{}: {}/{} ticks",
                result.status, result.completed_ticks, result.expected_ticks
            );
            if let Some(error) = result.error {
                return Err(error.into());
            }
        }
        Some("compare") if args.len() == 4 => {
            let report = compare_runs(Path::new(&args[1]), Path::new(&args[2]))?;
            report.write(Path::new(&args[3]))?;
            println!(
                "{}/{}: {}; report={}",
                report.case, report.scale, report.status, args[3]
            );
        }
        Some("compare") if args.len() == 5 => {
            let report = compare_performance_runs([
                Path::new(&args[1]),
                Path::new(&args[2]),
                Path::new(&args[3]),
            ])?;
            report.write(Path::new(&args[4]))?;
            println!(
                "{}/{}: {}; report={}",
                report.case, report.scale, report.status, args[4]
            );
        }
        _ => return Err(usage().into()),
    }
    Ok(())
}

fn usage() -> &'static str {
    "usage: laneflow-urban-harness plan <artifacts> <plan.toml> [--case CASE] [--probe-warm-up N --probe-ticks N | --performance] | run <artifacts> <plan.toml> <new-output> | compare <run-a> <run-b> <new-comparison.json> | compare <performance-a> <performance-b> <performance-c> <new-performance-comparison.toml>"
}
