use std::num::NonZeroU32;
use std::path::Path;
use std::process::ExitCode;

use laneflow_runtime::ExecutionConfig;
use laneflow_synth_harness::Error;
use laneflow_synth_harness::verification_cost::{Mode, run};

#[cfg(feature = "verification-cost-counts")]
#[global_allocator]
static ALLOCATOR: &stats_alloc::StatsAlloc<std::alloc::System> = &stats_alloc::INSTRUMENTED_SYSTEM;

const USAGE: &str = "synth-verification-cost run <input-directory> <implementation-commit> <workers> <new-output> --mode timing|counts";

fn execute() -> Result<(), Error> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 1 && args[0] == "--help" {
        println!("{USAGE}");
        return Ok(());
    }
    if args.len() != 7 || args[0] != "run" || args[5] != "--mode" {
        return Err(Error::Invalid(USAGE.into()));
    }
    let text = |index: usize| {
        args[index]
            .to_str()
            .ok_or_else(|| Error::Invalid("参数必须是 UTF-8".into()))
    };
    let workers = text(3)?
        .parse::<NonZeroU32>()
        .map_err(|_| Error::Invalid("workers 必须为非零整数".into()))?;
    let mode = match text(6)? {
        "timing" => Mode::Timing,
        "counts" => Mode::Counts,
        _ => return Err(Error::Invalid(USAGE.into())),
    };
    let report = run(
        Path::new(&args[1]),
        text(2)?,
        ExecutionConfig::new(workers),
        mode,
        Path::new(&args[4]),
    )?;
    println!("{}", serde_json::to_string(&report["complete"])?);
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
