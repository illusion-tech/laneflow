use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use issue_296_road_editing_source_calibration::{
    P100_PROFILE_COMBINATIONS, build_base_modules, build_regularity_probe_modules,
    compile_encoded_modules, encode_modules, load_bound_seed,
};
use laneflow_compiler::road_editing::RoadEditingModuleInput;
use laneflow_compiler::{
    CompilationOutput, CompilationUnitBuilder, CompileLimits, Compiler, GeometryAccuracyProfile,
    GeometryDirectionProfile,
};

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
        "road-editing-cross-language" => run_cross_language_fixtures(&arguments[1..]),
        _ => Err(format!(
            "unknown subcommand {command:?}; expected seed-audit, road-editing-p100, road-editing-regularity, road-editing-fixture-identities or road-editing-cross-language"
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

fn run_cross_language_fixtures(arguments: &[String]) -> Result<(), Box<dyn Error>> {
    let [cpp_path, csharp_path] = arguments else {
        return Err("road-editing-cross-language requires C++ and C# fixture paths".into());
    };
    let cpp_bytes = fs::read(cpp_path)?;
    let csharp_bytes = fs::read(csharp_path)?;
    let cpp = compile_cross_language_fixture("C++", &cpp_bytes)?;
    let csharp = compile_cross_language_fixture("C#", &csharp_bytes)?;
    let cpp_fingerprint = cpp.metrics().semantic_fingerprint();
    let csharp_fingerprint = csharp.metrics().semantic_fingerprint();
    if cpp_fingerprint != csharp_fingerprint {
        return Err(format!(
            "cross-language semantic fingerprints differ: cpp={} csharp={}",
            hex(&cpp_fingerprint),
            hex(&csharp_fingerprint)
        )
        .into());
    }
    let cpp_frame = only_cross_language_frame("C++", &cpp)?;
    let csharp_frame = only_cross_language_frame("C#", &csharp)?;
    if cpp_frame != csharp_frame {
        return Err("cross-language CanonicalFrame StableIds differ".into());
    }
    println!(
        "cross-language fixtures accepted cpp_bytes={} csharp_bytes={} semantic_fingerprint={}",
        cpp_bytes.len(),
        csharp_bytes.len(),
        hex(&cpp_fingerprint)
    );
    Ok(())
}

fn compile_cross_language_fixture(
    language: &str,
    bytes: &[u8],
) -> Result<CompilationOutput, Box<dyn Error>> {
    let limits = CompileLimits::p100_initial_v2();
    let input = RoadEditingModuleInput::try_new("cross-language", bytes, None)
        .map_err(|error| format!("{language} fixture identity is invalid: {error}"))?;
    let mut builder = CompilationUnitBuilder::new(limits);
    builder.add_road_editing_module(input)?;
    let unit = builder.build()?;
    Ok(Compiler::new().compile(unit)?)
}

fn only_cross_language_frame(
    language: &str,
    output: &CompilationOutput,
) -> Result<[u8; 16], Box<dyn Error>> {
    let mut frames = output.lir().canonical_frames();
    if frames.len() != 1 {
        return Err(format!(
            "{language} fixture produced {} CanonicalFrames; expected one",
            frames.len()
        )
        .into());
    }
    Ok(*frames
        .next()
        .expect("length check proves one CanonicalFrame")
        .stable_id()
        .as_untyped()
        .as_bytes())
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
        "compiled lane_edges={} facility_bands={} source_bytes={} verified_tables={} geometry_points={} regularity_visits={} regularity_max={} frontend_peak_bytes={} lir_records={} logical_bytes={} combined_peak_bytes={} semantic_fingerprint={}",
        output.lir().lane_edges().count(),
        output.lir().facility_bands().count(),
        metrics.source_bytes_total(),
        metrics.verified_table_occurrence_count(),
        metrics.geometry_point_count(),
        metrics.total_horizontal_regularity_node_visits(),
        metrics.maximum_horizontal_regularity_node_visits_per_offset_bearing_source_segment(),
        metrics.frontend_controlled_peak_bytes(),
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
