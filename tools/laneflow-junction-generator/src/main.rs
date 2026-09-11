use std::path::PathBuf;

use laneflow_junction_generator::{check_files, generate_files};

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
        "{command} ok: {} edges, {} movements, {} maneuver paths, {} maneuver gates, \
         {} stop lines, {} waiting zones, {} conflict zones, {} streams, {} signal groups, \
         {} phases, {} routes, {} spawn slots",
        counts.edges,
        counts.movements,
        counts.maneuver_paths,
        counts.maneuver_gates,
        counts.stop_lines,
        counts.waiting_zones,
        counts.conflict_zones,
        counts.streams,
        counts.signal_groups,
        counts.phases,
        counts.routes,
        counts.spawn_slots
    );
    Ok(())
}

fn usage() -> String {
    "usage: laneflow-junction-generator <generate|check> --config <path>".to_owned()
}
