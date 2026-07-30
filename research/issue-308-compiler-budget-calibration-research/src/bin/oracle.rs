mod support;

use issue_308_compiler_budget_calibration_research::{
    build_corridor_known_vectors, build_current_fixtures_known_vectors,
    build_identity_known_vectors, build_identity_oracle_child, build_junction_grid_known_vectors,
    build_module_graph_known_vectors, load_repository_contract, oracle_binary_descriptor,
    verify_corridor_oracle_matrix, verify_current_fixtures_oracle, verify_identity_oracle_matrix,
    verify_junction_grid_oracle_matrix,
};
use serde::Serialize;
use std::path::Path;

const USAGE: &str = "用法：issue-308-compiler-budget-calibration-oracle <describe-role|run|verify-matrix|write-known-vectors>\n  run <graph-profile> <N>";

fn main() {
    support::main_with(run);
}

fn run() -> Result<(), String> {
    let mut arguments = std::env::args_os();
    let _binary = arguments.next();
    let command = support::next_utf8_argument(&mut arguments, "command", USAGE)?;
    match command.as_str() {
        "describe-role" => {
            support::require_no_more_arguments(&mut arguments, USAGE)?;
            support::print_json(&oracle_binary_descriptor(), "预言机角色描述")
        }
        "run" => {
            let graph_profile = support::parse_graph_profile(&support::next_utf8_argument(
                &mut arguments,
                "graph-profile",
                USAGE,
            )?)?;
            let n = support::parse_positive_u32(
                &support::next_utf8_argument(&mut arguments, "N", USAGE)?,
                "N",
            )?;
            support::require_no_more_arguments(&mut arguments, USAGE)?;

            let trusted = load_repository_contract().map_err(|error| error.to_string())?;
            let report = build_identity_oracle_child(&trusted, graph_profile, n)
                .map_err(|error| error.to_string())?;
            support::print_json(&report, "独立预言机结果")
        }
        "verify-matrix" => {
            support::require_no_more_arguments(&mut arguments, USAGE)?;
            let trusted = load_repository_contract().map_err(|error| error.to_string())?;
            let identity =
                verify_identity_oracle_matrix(&trusted).map_err(|error| error.to_string())?;
            let corridor =
                verify_corridor_oracle_matrix(&trusted).map_err(|error| error.to_string())?;
            let junction_grid =
                verify_junction_grid_oracle_matrix(&trusted).map_err(|error| error.to_string())?;
            let current_fixtures =
                verify_current_fixtures_oracle(&trusted).map_err(|error| error.to_string())?;
            let report = OracleMatrixReport {
                identity,
                corridor,
                junction_grid,
                current_fixtures,
            };
            support::print_json(&report, "预言机矩阵验证结果")
        }
        "write-known-vectors" => {
            support::require_no_more_arguments(&mut arguments, USAGE)?;
            write_known_vectors()
        }
        _ => Err(USAGE.to_owned()),
    }
}

fn write_known_vectors() -> Result<(), String> {
    let trusted = load_repository_contract().map_err(|error| error.to_string())?;
    let oracle_report =
        verify_identity_oracle_matrix(&trusted).map_err(|error| error.to_string())?;
    let corridor_oracle_report =
        verify_corridor_oracle_matrix(&trusted).map_err(|error| error.to_string())?;
    let junction_grid_oracle_report =
        verify_junction_grid_oracle_matrix(&trusted).map_err(|error| error.to_string())?;
    let current_fixtures_oracle_report =
        verify_current_fixtures_oracle(&trusted).map_err(|error| error.to_string())?;
    let generator_contract = trusted
        .generator_contract()
        .map_err(|error| error.to_string())?;
    let identity_contract = trusted
        .identity_contract()
        .map_err(|error| error.to_string())?;
    let module_vectors = build_module_graph_known_vectors(
        &generator_contract,
        &trusted.descriptor.workload_manifest.sha256,
    )
    .map_err(|error| error.to_string())?;
    let identity_vectors = build_identity_known_vectors(
        &generator_contract,
        &identity_contract,
        &trusted.descriptor.workload_manifest.sha256,
    )
    .map_err(|error| error.to_string())?;
    let corridor_vectors =
        build_corridor_known_vectors(&trusted).map_err(|error| error.to_string())?;
    let junction_grid_vectors =
        build_junction_grid_known_vectors(&trusted).map_err(|error| error.to_string())?;
    let current_fixture_vectors =
        build_current_fixtures_known_vectors(&trusted).map_err(|error| error.to_string())?;
    let output_directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("known-vectors");
    std::fs::create_dir_all(&output_directory).map_err(|error| {
        format!(
            "无法创建已知向量目录 {}：{error}",
            output_directory.display()
        )
    })?;
    write_known_vector(
        &output_directory.join("module-graphs-v1.json"),
        &module_vectors,
    )?;
    write_known_vector(
        &output_directory.join("identity-records-v1.json"),
        &identity_vectors,
    )?;
    write_known_vector(
        &output_directory.join("corridor-summary-v1.json"),
        &corridor_vectors,
    )?;
    write_known_vector(
        &output_directory.join("junction-grid-summary-v1.json"),
        &junction_grid_vectors,
    )?;
    write_known_vector(
        &output_directory.join("current-fixtures-summary-v1.json"),
        &current_fixture_vectors,
    )?;
    println!("written={}", output_directory.display());
    println!("oracleCheckedCases={}", oracle_report.checked_cases);
    println!(
        "corridorOracleCheckedCases={}",
        corridor_oracle_report.checked_cases
    );
    println!(
        "junctionGridOracleCheckedCases={}",
        junction_grid_oracle_report.checked_cases
    );
    println!(
        "currentFixturesOracleCheckedCases={}",
        current_fixtures_oracle_report.checked_cases
    );
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OracleMatrixReport {
    identity: issue_308_compiler_budget_calibration_research::OracleVerificationReport,
    corridor: issue_308_compiler_budget_calibration_research::CorridorOracleVerificationReport,
    junction_grid:
        issue_308_compiler_budget_calibration_research::JunctionGridOracleVerificationReport,
    current_fixtures:
        issue_308_compiler_budget_calibration_research::CurrentFixturesOracleVerificationReport,
}

fn write_known_vector(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let json = serde_json::to_string_pretty(value)
        .map_err(|error| format!("无法序列化已知向量 {}：{error}", path.display()))?;
    std::fs::write(path, format!("{json}\n"))
        .map_err(|error| format!("无法写入已知向量 {}：{error}", path.display()))
}
