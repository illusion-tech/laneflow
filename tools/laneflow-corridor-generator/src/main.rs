use std::path::PathBuf;

use laneflow_corridor_generator::{check_files, generate_files};

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut arguments = std::env::args_os().skip(1);
    let command = arguments
        .next()
        .and_then(|value| value.into_string().ok())
        .ok_or_else(usage)?;
    let flag = arguments
        .next()
        .and_then(|value| value.into_string().ok())
        .ok_or_else(usage)?;
    let path = arguments.next().map(PathBuf::from).ok_or_else(usage)?;
    if flag != "--config" || arguments.next().is_some() {
        return Err(usage());
    }

    let counts = match command.as_str() {
        "generate" => generate_files(&path),
        "check" => check_files(&path),
        _ => return Err(usage()),
    }
    .map_err(|error| error.to_string())?;
    println!(
        "{command} ok: {} edges, {} routes, {} stop lines, {} spawn slots",
        counts.edges, counts.routes, counts.stop_lines, counts.spawn_slots
    );
    Ok(())
}

fn usage() -> String {
    "usage: laneflow-corridor-generator <generate|check> --config <path>".to_owned()
}
