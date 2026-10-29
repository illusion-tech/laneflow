use std::{path::Path, process::ExitCode};

use laneflow_synth_harness::{Error, measurement};

#[cfg(feature = "allocation")]
#[global_allocator]
static ALLOCATOR: &stats_alloc::StatsAlloc<std::alloc::System> = &stats_alloc::INSTRUMENTED_SYSTEM;

fn execute() -> Result<(), Error> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let text = |index: usize| {
        args[index]
            .to_str()
            .ok_or_else(|| Error::Invalid("参数必须是 UTF-8".into()))
    };
    if args.len() == 1 && args[0] == "--help" {
        println!(
            "measure run <input-directory> <oracle-directory> <implementation-commit> <environment.json> <percent> <round> <new-output>\nmeasure prepare-oracles <semantic-binary> <input-root> <implementation-commit> <new-output>\nmeasure matrix|stretch-matrix <input-root> <oracle-root> <implementation-commit> <environment.json> <new-output>\nmeasure guardrail <W1-input> <new-output>"
        );
        return Ok(());
    }
    if args.len() == 3 && args[0] == "guardrail" {
        measurement::guardrail(Path::new(&args[1]), Path::new(&args[2]))?;
        return Ok(());
    }
    if args.len() == 5 && args[0] == "prepare-oracles" {
        return measurement::prepare_oracles(
            Path::new(&args[1]),
            Path::new(&args[2]),
            text(3)?,
            Path::new(&args[4]),
        );
    }
    if args.len() == 6 && (args[0] == "matrix" || args[0] == "stretch-matrix") {
        return measurement::matrix(
            Path::new(&args[1]),
            Path::new(&args[2]),
            text(3)?,
            Path::new(&args[4]),
            Path::new(&args[5]),
            args[0] == "stretch-matrix",
        );
    }
    if args.len() != 8 || args[0] != "run" {
        return Err(Error::Invalid("参数错误；使用 --help".into()));
    }
    let text = |index: usize| {
        args[index]
            .to_str()
            .ok_or_else(|| Error::Invalid("参数必须是 UTF-8".into()))
    };
    let percent = text(5)?
        .parse()
        .map_err(|_| Error::Invalid("比例非法".into()))?;
    let round = text(6)?
        .parse()
        .map_err(|_| Error::Invalid("轮次非法".into()))?;
    let report = measurement::run(
        Path::new(&args[1]),
        Path::new(&args[2]),
        text(3)?,
        Path::new(&args[4]),
        percent,
        round,
        Path::new(&args[7]),
    )?;
    println!("{}", serde_json::to_string(&report)?);
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
