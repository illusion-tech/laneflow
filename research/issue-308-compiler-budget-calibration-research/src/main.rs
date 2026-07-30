use issue_308_compiler_budget_calibration_research::{
    CONTRACT_DESCRIPTOR_BYTE_LENGTH, build_module_graph_known_vectors, load_repository_contract,
};

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
        _ => Err(usage()),
    }
}

fn usage() -> String {
    "用法：issue-308-compiler-budget-calibration-research <verify-contract|print-module-graph-known-vectors>".to_owned()
}
