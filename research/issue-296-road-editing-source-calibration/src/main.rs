use std::path::Path;

use issue_296_road_editing_source_calibration::load_bound_seed;

fn main() {
    let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("research crate is two levels below repository root");
    match load_bound_seed(repository_root) {
        Ok(audit) => println!(
            "validated {} modules, {} declarations and {} curve segments",
            audit.module_count, audit.stable_declaration_count, audit.curve_segment_count
        ),
        Err(error) => {
            eprintln!("road-editing P100 seed audit failed: {error}");
            std::process::exit(1);
        }
    }
}
