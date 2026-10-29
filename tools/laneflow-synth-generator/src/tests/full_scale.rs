use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::json;

use super::assert_runtime_consumption;
use crate::encoding::canonical_json;
use crate::{Manifest, verify_directory};

fn generate_process(
    binary: &Path,
    directory: &Path,
    scale: u32,
    case: &str,
    delta: u32,
    source: &str,
    generator: &str,
) -> Manifest {
    let output = Command::new(binary)
        .args([
            "generate",
            &scale.to_string(),
            case,
            &delta.to_string(),
            source,
            generator,
        ])
        .arg(directory)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "generator failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// #1156：两个全新进程覆盖完整矩阵，结果与原始输入保留在调用者指定的新目录。
#[test]
#[ignore = "正式规模交付验收：需 release 二进制、来源提交和全新证据目录"]
fn full_scale_independent_process_matrix() {
    let binary =
        PathBuf::from(std::env::var_os("LF_SYNTH_GENERATOR_BIN").expect("LF_SYNTH_GENERATOR_BIN"));
    let root =
        PathBuf::from(std::env::var_os("LF_SYNTH_EVIDENCE_ROOT").expect("LF_SYNTH_EVIDENCE_ROOT"));
    let source = std::env::var("LF_SYNTH_SOURCE_COMMIT").expect("LF_SYNTH_SOURCE_COMMIT");
    let generator = std::env::var("LF_SYNTH_GENERATOR_COMMIT").expect("LF_SYNTH_GENERATOR_COMMIT");
    fs::create_dir(&root).unwrap();
    let mut rows = Vec::new();
    for scale in [10_000, 100_000] {
        for case in ["W1", "W2", "W3", "W4"] {
            for delta in [16, 33] {
                let label = format!("{scale}-{case}-{delta}");
                let first = root.join(format!("{label}-a"));
                let second = root.join(format!("{label}-b"));
                let manifest =
                    generate_process(&binary, &first, scale, case, delta, &source, &generator);
                let repeated =
                    generate_process(&binary, &second, scale, case, delta, &source, &generator);
                assert_eq!(manifest, repeated);
                for name in [
                    "network.lfca",
                    "state.json",
                    "input-plan.json",
                    "manifest.json",
                ] {
                    assert_eq!(
                        fs::read(first.join(name)).unwrap(),
                        fs::read(second.join(name)).unwrap(),
                        "independent bytes: {label}/{name}"
                    );
                }
                let checked = verify_directory(&first).unwrap();
                assert_runtime_consumption(&checked);
                rows.push(
                    json!({"manifest":manifest,"independent_process_byte_equality":true,
                    "canonical_rebuild_verified":true,"static_counts_and_geometry_verified":true,
                    "runtime_initial_population_and_one_step_verified":true}),
                );
                let summary = json!({"format":"lf-synth-generation-evidence-1","complete":rows.len()==16,
                    "source_commit":source,"generator_commit":generator,"rows":rows,
                    "scope":"input_generation_only_not_execution_harness_or_product_certification"});
                fs::write(root.join("summary.json"), canonical_json(&summary).unwrap()).unwrap();
                eprintln!("verified {label} ({}/16)", rows.len());
            }
        }
    }
}
