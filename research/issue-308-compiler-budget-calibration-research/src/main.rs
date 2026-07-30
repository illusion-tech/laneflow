use issue_308_compiler_budget_calibration_research::{
    CONTRACT_DESCRIPTOR_BYTE_LENGTH, build_identity_known_vectors,
    build_module_graph_known_vectors, load_repository_contract, verify_identity_oracle_matrix,
};
use serde::Serialize;
use std::path::Path;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut arguments = std::env::args_os();
    let _binary = arguments.next();
    let command = arguments
        .next()
        .and_then(|value| value.into_string().ok())
        .ok_or_else(usage)?;
    if arguments.next().is_some() {
        return Err(usage());
    }

    match command.as_str() {
        "verify-contract" => {
            let contract = load_repository_contract().map_err(|error| error.to_string())?;
            println!("contractSchema={}", contract.descriptor.schema);
            println!(
                "contractSchemaVersion={}",
                contract.descriptor.schema_version
            );
            println!("contractByteLength={CONTRACT_DESCRIPTOR_BYTE_LENGTH}");
            println!("contractSha256={}", contract.descriptor_sha256);
            println!(
                "workloadManifestByteLength={}",
                contract.descriptor.workload_manifest.byte_length
            );
            println!(
                "workloadManifestSha256={}",
                contract.descriptor.workload_manifest.sha256
            );
            println!(
                "evidenceSchemaByteLength={}",
                contract.descriptor.evidence_schema.byte_length
            );
            println!(
                "evidenceSchemaSha256={}",
                contract.descriptor.evidence_schema.sha256
            );
            Ok(())
        }
        "print-module-graph-known-vectors" => {
            let trusted = load_repository_contract().map_err(|error| error.to_string())?;
            let generator_contract = trusted
                .generator_contract()
                .map_err(|error| error.to_string())?;
            let vectors = build_module_graph_known_vectors(
                &generator_contract,
                &trusted.descriptor.workload_manifest.sha256,
            )
            .map_err(|error| error.to_string())?;
            let json = serde_json::to_string_pretty(&vectors)
                .map_err(|error| format!("无法序列化模块图已知向量：{error}"))?;
            println!("{json}");
            Ok(())
        }
        "print-identity-known-vectors" => {
            let trusted = load_repository_contract().map_err(|error| error.to_string())?;
            let generator_contract = trusted
                .generator_contract()
                .map_err(|error| error.to_string())?;
            let identity_contract = trusted
                .identity_contract()
                .map_err(|error| error.to_string())?;
            let vectors = build_identity_known_vectors(
                &generator_contract,
                &identity_contract,
                &trusted.descriptor.workload_manifest.sha256,
            )
            .map_err(|error| error.to_string())?;
            let json = serde_json::to_string_pretty(&vectors)
                .map_err(|error| format!("无法序列化身份已知向量：{error}"))?;
            println!("{json}");
            Ok(())
        }
        "write-known-vectors" => {
            let trusted = load_repository_contract().map_err(|error| error.to_string())?;
            let oracle_report =
                verify_identity_oracle_matrix(&trusted).map_err(|error| error.to_string())?;
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
            println!("written={}", output_directory.display());
            println!("oracleCheckedCases={}", oracle_report.checked_cases);
            Ok(())
        }
        "verify-identity-oracle" => {
            let trusted = load_repository_contract().map_err(|error| error.to_string())?;
            let report =
                verify_identity_oracle_matrix(&trusted).map_err(|error| error.to_string())?;
            println!("checkedCases={}", report.checked_cases);
            println!("checkedN1Cases={}", report.checked_n1_cases);
            println!("checkedN2Cases={}", report.checked_n2_cases);
            println!("checkedStageCases={}", report.checked_stage_cases);
            Ok(())
        }
        _ => Err(usage()),
    }
}

fn write_known_vector(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let json = serde_json::to_string_pretty(value)
        .map_err(|error| format!("无法序列化已知向量 {}：{error}", path.display()))?;
    std::fs::write(path, format!("{json}\n"))
        .map_err(|error| format!("无法写入已知向量 {}：{error}", path.display()))
}

fn usage() -> String {
    "用法：issue-308-compiler-budget-calibration-research <verify-contract|print-module-graph-known-vectors|print-identity-known-vectors|write-known-vectors|verify-identity-oracle>".to_owned()
}
