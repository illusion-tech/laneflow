//! #779 复用共享平衡采集器，对空转移区间短路做无插桩整步 A/B。
#[allow(dead_code)]
mod cache_research;
use cache_research::io;
mod empty_ranges_route;

use std::{error::Error, path::Path};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const BASE: &str = "b67c6ea9d99a8ea806fb31ede0364978e0e09618";
const EXPERIMENT: cache_research::Experiment = cache_research::Experiment {
    baseline: BASE,
    protocol: "empty-transition-ranges-abba-v1",
    count_p2: false,
};

fn need(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(message.to_owned().into())
    }
}

fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("route-run") if args.len() == 3 => {
            empty_ranges_route::capture(Path::new(&args[1]), Path::new(&args[2]))
        }
        Some("route-analyze" | "route-verify") if args.len() == 3 => {
            let result = empty_ranges_route::analyze(Path::new(&args[1]))?;
            if args[0] == "route-verify" {
                need(result == io::read_json(Path::new(&args[2]))?, "route published mismatch")
            } else {
                io::outside(Path::new(&args[1]), Path::new(&args[2]))?;
                io::write_new(Path::new(&args[2]), &result)
            }
        }
        Some("prepare") if args.len() == 5 && args[2] == "plain" => {
            cache_research::export_for(
                Path::new(&args[4]),
                &args[1],
                &args[2],
                &args[3],
                EXPERIMENT,
            )
        }
        Some("run") if args.len() == 5 && args[1] == "plain" => cache_research::capture_for(
            &args[1],
            Path::new(&args[2]),
            Path::new(&args[3]),
            Path::new(&args[4]),
            EXPERIMENT,
        ),
        Some("analyze" | "verify") if args.len() == 3 => {
            let raw = Path::new(&args[1]);
            let output = Path::new(&args[2]);
            let value = cache_research::analyze_for(raw, EXPERIMENT)?;
            if args[0] == "verify" {
                need(value == io::read_json(output)?, "published mismatch")?;
                println!("verified");
                Ok(())
            } else {
                io::outside(raw, output)?;
                io::write_new(output, &value)
            }
        }
        _ => Err(
            "prepare <base|candidate> plain <commit> <root> | run plain <root> <inputs> <new-raw> | analyze|verify <raw> <results>"
                .into(),
        ),
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
