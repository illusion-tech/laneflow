use std::path::PathBuf;

use laneflow_lust_converter::{LUST_COMMIT, LUST_REPOSITORY, LUST_TAG, convert, verify_source};

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

    match command.as_str() {
        "verify-source" => {
            let flag = arguments
                .next()
                .and_then(|value| value.into_string().ok())
                .ok_or_else(usage)?;
            let path = arguments.next().map(PathBuf::from).ok_or_else(usage)?;
            if flag != "--source-dir" || arguments.next().is_some() {
                return Err(usage());
            }
            let verified = verify_source(&path).map_err(|error| error.to_string())?;
            println!(
                "verify-source ok: {} pinned files under {} (LuST {} @ {})",
                verified.files.len(),
                verified.source_dir.display(),
                LUST_TAG,
                &LUST_COMMIT[..12]
            );
            println!("repository: {LUST_REPOSITORY}");
            for file in &verified.files {
                println!(
                    "  {}  {}  sha256:{}",
                    file.bytes, file.relative_path, file.sha256_hex
                );
            }
            Ok(())
        }
        "convert" => {
            let flag = arguments
                .next()
                .and_then(|value| value.into_string().ok())
                .ok_or_else(usage)?;
            let path = arguments.next().map(PathBuf::from).ok_or_else(usage)?;
            if flag != "--config" || arguments.next().is_some() {
                return Err(usage());
            }
            let outputs = convert(&path).map_err(|error| error.to_string())?;
            println!(
                "convert ok: wrote static/source bundles under {}",
                outputs.output_dir.display()
            );
            println!("  {}", outputs.traffic.display());
            println!("  {}", outputs.spatial.display());
            println!("  {}", outputs.manifest.display());
            println!("  {}", outputs.conversion_report.display());
            println!("  {}", outputs.population.display());
            println!("  {}", outputs.source_tar.display());
            println!("  {}", outputs.static_tar.display());
            println!("  {}", outputs.semantic_provenance.display());
            println!("  {}", outputs.build_provenance.display());
            Ok(())
        }
        _ => Err(usage()),
    }
}

fn usage() -> String {
    "usage: laneflow-lust-converter <verify-source --source-dir <path> | convert --config <path>>"
        .to_owned()
}
