use std::error::Error;
use std::path::{Path, PathBuf};

use issue_296_road_editing_source_calibration::{
    P100_PROFILE_COMBINATIONS, build_base_modules, build_regularity_probe_modules,
    compile_encoded_modules, encode_modules, load_bound_seed,
};
use laneflow_compiler::{CompileLimits, GeometryAccuracyProfile, GeometryDirectionProfile};

fn main() {
    let repository_root = repository_root();
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let command = arguments.first().map_or("seed-audit", String::as_str);
    let result = match command {
        "seed-audit" => require_no_arguments(&arguments[1..])
            .and_then(|()| run_seed_audit(&repository_root)),
        "road-editing-p100" => parse_profile_arguments(&arguments[1..])
            .and_then(|(accuracy, direction)| run_base_p100(&repository_root, accuracy, direction)),
        "road-editing-regularity" => require_no_arguments(&arguments[1..])
            .and_then(|()| run_regularity_probe(&repository_root)),
        "road-editing-fixture-identities" => require_no_arguments(&arguments[1..])
            .and_then(|()| run_fixture_identities(&repository_root)),
        _ => Err(format!(
            "unknown subcommand {command:?}; expected seed-audit, road-editing-p100, road-editing-regularity or road-editing-fixture-identities"
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

fn run_base_p100(
    repository_root: &Path,
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
) -> Result<(), Box<dyn Error>> {
    let limits = CompileLimits::p100_initial_v2();
    let modules = build_base_modules(repository_root, accuracy, direction, &limits)?;
    print_workload("LF-ROAD-EDITING-P100-v1", modules, limits)
}

fn run_regularity_probe(repository_root: &Path) -> Result<(), Box<dyn Error>> {
    let limits = CompileLimits::p100_initial_v2();
    let modules = build_regularity_probe_modules(repository_root, &limits)?;
    print_workload("LF-ROAD-EDITING-P100-REGULARITY-v1", modules, limits)
}

fn run_fixture_identities(repository_root: &Path) -> Result<(), Box<dyn Error>> {
    let limits = CompileLimits::p100_initial_v2();
    for combination in P100_PROFILE_COMBINATIONS {
        let modules = build_base_modules(
            repository_root,
            combination.accuracy(),
            combination.direction(),
            &limits,
        )?;
        print_fixture_identities("LF-ROAD-EDITING-P100-v1", modules, &limits)?;
    }
    print_fixture_identities(
        "LF-ROAD-EDITING-P100-REGULARITY-v1",
        build_regularity_probe_modules(repository_root, &limits)?,
        &limits,
    )?;
    Ok(())
}

fn print_workload(
    workload: &str,
    modules: Vec<issue_296_road_editing_source_calibration::TypedP100Module>,
    limits: CompileLimits,
) -> Result<(), Box<dyn Error>> {
    let accuracy = modules
        .first()
        .ok_or("generated workload has no modules")?
        .module()
        .geometry_accuracy_profile();
    let direction = modules[0].module().geometry_direction_profile();
    let encoded = encode_modules(modules, &limits)?;
    for module in &encoded {
        println!(
            "workload={workload} accuracy={} direction={} module={} document={} bytes={} retained={} sha256={}",
            accuracy as u8,
            direction as u8,
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

fn print_fixture_identities(
    workload: &str,
    modules: Vec<issue_296_road_editing_source_calibration::TypedP100Module>,
    limits: &CompileLimits,
) -> Result<(), Box<dyn Error>> {
    let accuracy = modules
        .first()
        .ok_or("generated workload has no modules")?
        .module()
        .geometry_accuracy_profile();
    let direction = modules[0].module().geometry_direction_profile();
    for module in encode_modules(modules, limits)? {
        println!(
            "workload={workload} accuracy={} direction={} module={} bytes={} sha256={}",
            accuracy as u8,
            direction as u8,
            module.namespace(),
            module.as_bytes().len(),
            hex(&module.sha256())
        );
    }
    Ok(())
}

fn require_no_arguments(arguments: &[String]) -> Result<(), Box<dyn Error>> {
    if arguments.is_empty() {
        Ok(())
    } else {
        Err(format!("unexpected arguments: {arguments:?}").into())
    }
}

fn parse_profile_arguments(
    arguments: &[String],
) -> Result<(GeometryAccuracyProfile, GeometryDirectionProfile), Box<dyn Error>> {
    let [accuracy, direction] = arguments else {
        return Err(
            "road-editing-p100 requires numeric accuracy and direction profile codes".into(),
        );
    };
    Ok((
        match accuracy.as_str() {
            "1" => GeometryAccuracyProfile::Fine2Cm,
            "2" => GeometryAccuracyProfile::Balanced5Cm,
            "3" => GeometryAccuracyProfile::Compact10Cm,
            _ => return Err(format!("unknown geometry accuracy profile code {accuracy:?}").into()),
        },
        match direction.as_str() {
            "1" => GeometryDirectionProfile::Smooth1Deg,
            "2" => GeometryDirectionProfile::Balanced2Deg,
            "3" => GeometryDirectionProfile::Compact5Deg,
            _ => {
                return Err(
                    format!("unknown geometry direction profile code {direction:?}").into(),
                );
            }
        },
    ))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        write!(output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}
