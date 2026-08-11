use std::error::Error;
use std::path::{Path, PathBuf};

use issue_296_road_editing_source_calibration::{
    build_base_modules, compile_encoded_modules, encode_modules, load_bound_seed,
};
use laneflow_compiler::{CompileLimits, GeometryAccuracyProfile, GeometryDirectionProfile};

fn main() {
    let repository_root = repository_root();
    let command = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "seed-audit".to_owned());
    let result = match command.as_str() {
        "seed-audit" => run_seed_audit(&repository_root),
        "road-editing-p100" => run_base_p100(&repository_root),
        _ => Err(format!(
            "unknown subcommand {command:?}; expected seed-audit or road-editing-p100"
        )
        .into()),
    };
    if let Err(error) = result {
        eprintln!("road-editing P100 calibration failed: {error}");
        std::process::exit(1);
    }
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("research crate is two levels below repository root")
        .to_path_buf()
}

fn run_seed_audit(repository_root: &Path) -> Result<(), Box<dyn Error>> {
    let audit = load_bound_seed(repository_root)?;
    println!(
        "validated {} modules, {} declarations and {} curve segments",
        audit.module_count, audit.stable_declaration_count, audit.curve_segment_count
    );
    Ok(())
}

fn run_base_p100(repository_root: &Path) -> Result<(), Box<dyn Error>> {
    let limits = CompileLimits::p100_initial_v2();
    let modules = build_base_modules(
        repository_root,
        GeometryAccuracyProfile::Balanced5Cm,
        GeometryDirectionProfile::Balanced2Deg,
        &limits,
    )?;
    let encoded = encode_modules(modules, &limits)?;
    for module in &encoded {
        println!(
            "module={} document={} bytes={} retained={} sha256={}",
            module.namespace(),
            module.source_document_key(),
            module.as_bytes().len(),
            module.retained_capacity_bytes(),
            hex(&module.sha256())
        );
    }
    let output = compile_encoded_modules(&encoded, limits)?;
    let metrics = output.metrics();
    println!(
        "compiled lane_edges={} facility_bands={} lir_records={} logical_bytes={} peak_bytes={} semantic_fingerprint={}",
        output.lir().lane_edges().count(),
        output.lir().facility_bands().count(),
        metrics.lir_record_count(),
        metrics.output_logical_bytes(),
        metrics.compiler_controlled_peak_bytes(),
        hex(&metrics.semantic_fingerprint())
    );
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        write!(output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}
