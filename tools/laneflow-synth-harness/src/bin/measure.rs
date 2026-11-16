use std::{path::Path, process::ExitCode};

use laneflow_synth_harness::{Error, measurement};

#[cfg(feature = "allocation")]
#[global_allocator]
static ALLOCATOR: &stats_alloc::StatsAlloc<std::alloc::System> = &stats_alloc::INSTRUMENTED_SYSTEM;

const NO_LOAD_SAMPLING: &str = "--no-load-sampling";

fn execute() -> Result<(), Error> {
    let mut args: Vec<_> = std::env::args_os().skip(1).collect();
    // lf-synth-measurement.md §7.3：逐段负载采样默认开启；末尾 `--no-load-sampling`
    // 关闭，仅用于采样开销对比，两种配置的计时不得合并统计。
    let load_sampling = if args.last().is_some_and(|arg| arg == NO_LOAD_SAMPLING) {
        args.pop();
        false
    } else {
        true
    };
    let text = |index: usize| {
        args[index]
            .to_str()
            .ok_or_else(|| Error::Invalid("参数必须是 UTF-8".into()))
    };
    if args.len() == 1 && args[0] == "--help" {
        println!(
            "measure run <input-directory> <oracle-directory> <implementation-commit> <environment-before.json> <percent> <round> <new-output> [--no-load-sampling]\nmeasure prepare-oracles <semantic-binary> <input-root> <implementation-commit> <new-output>\nmeasure matrix <input-root> <oracle-root> <implementation-commit> <environment-input.json> <new-output> [--rows <label,...>] [--no-load-sampling]\nmeasure stretch-matrix <input-root> <oracle-root> <implementation-commit> <environment-input.json> <new-output> [--no-load-sampling]\nmeasure guardrail <W1-input> <new-output>"
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
    let subset = args.len() == 8 && args[0] == "matrix" && args[6] == "--rows";
    if subset || (args.len() == 6 && (args[0] == "matrix" || args[0] == "stretch-matrix")) {
        let selection = if subset {
            // 子集矩阵只用于采样开销对比与阈值校准，不是正式矩阵结果。
            measurement::RowSelection::Subset(text(7)?.split(',').map(str::to_owned).collect())
        } else if args[0] == "stretch-matrix" {
            measurement::RowSelection::Stretch
        } else {
            measurement::RowSelection::Nominal
        };
        return measurement::matrix(
            Path::new(&args[1]),
            Path::new(&args[2]),
            text(3)?,
            Path::new(&args[4]),
            Path::new(&args[5]),
            &selection,
            load_sampling,
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
        measurement::RunConfig {
            percent,
            round,
            load_sampling,
        },
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
