use laneflow_urban_generator::{Scale, UrbanConfig, compare_artifacts, generate};
use stats_alloc::{INSTRUMENTED_SYSTEM, StatsAlloc};
use std::path::PathBuf;

#[global_allocator]
static ALLOCATOR: &StatsAlloc<std::alloc::System> = &INSTRUMENTED_SYSTEM;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    const USAGE: &str = "laneflow-urban-generator --config PATH --scale fixture|10k|100k --output NEW_DIRECTORY\nlaneflow-urban-generator compare DIRECTORY_A DIRECTORY_B";
    let mut args = std::env::args().skip(1).peekable();
    if args.peek().is_some_and(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    if args.peek().is_some_and(|a| a == "compare") {
        args.next();
        let a = PathBuf::from(args.next().ok_or(USAGE)?);
        let b = PathBuf::from(args.next().ok_or(USAGE)?);
        if args.next().is_some() {
            return Err(USAGE.into());
        }
        compare_artifacts(&a, &b)?;
        println!("All eight canonical files are byte-identical (measurements excluded).");
        return Ok(());
    }
    let mut scale = None;
    let mut config = None;
    let mut output = None;
    while let Some(arg) = args.next() {
        let value = args.next().ok_or(USAGE)?;
        match arg.as_str() {
            "--scale" => scale = Some(Scale::parse(&value)?),
            "--config" => config = Some(PathBuf::from(value)),
            "--output" => output = Some(PathBuf::from(value)),
            _ => return Err(format!("unknown option: {arg}").into()),
        }
    }
    let scale = scale.ok_or("--scale is required")?;
    let config = UrbanConfig::parse(&std::fs::read_to_string(
        config.ok_or("--config is required")?,
    )?)?;
    let output = output.ok_or("--output is required")?;
    let manifest = generate(&config, scale, &output, Some(ALLOCATOR))?;
    println!(
        "LF-CN-URBAN {}: {} cells, {} tiles, revision {}",
        scale.name(),
        manifest.cells,
        manifest.tiles,
        manifest.network_revision
    );
    println!("Artifacts and validation report: {}", output.display());
    Ok(())
}
