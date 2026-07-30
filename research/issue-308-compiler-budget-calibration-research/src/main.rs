use issue_308_compiler_budget_calibration_research::{
    CONTRACT_DESCRIPTOR_BYTE_LENGTH, GraphProfileId, build_identity_known_vectors,
    build_module_graph_known_vectors, load_repository_contract, measure_identity_timing_child,
    run_identity_fresh_process_pilot, verify_identity_oracle_matrix,
};
use serde::Serialize;
use std::ffi::OsString;
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

    match command.as_str() {
        "verify-contract" => {
            require_no_more_arguments(&mut arguments)?;
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
            require_no_more_arguments(&mut arguments)?;
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
            require_no_more_arguments(&mut arguments)?;
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
            require_no_more_arguments(&mut arguments)?;
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
            require_no_more_arguments(&mut arguments)?;
            let trusted = load_repository_contract().map_err(|error| error.to_string())?;
            let report =
                verify_identity_oracle_matrix(&trusted).map_err(|error| error.to_string())?;
            println!("checkedCases={}", report.checked_cases);
            println!("checkedN1Cases={}", report.checked_n1_cases);
            println!("checkedN2Cases={}", report.checked_n2_cases);
            println!("checkedStageCases={}", report.checked_stage_cases);
            Ok(())
        }
        "identity-timing-child" => {
            let compiler_instance_id = next_utf8_argument(&mut arguments, "compiler-instance-id")?;
            let graph_profile =
                parse_graph_profile(&next_utf8_argument(&mut arguments, "graph-profile")?)?;
            let n = parse_positive_n(&next_utf8_argument(&mut arguments, "N")?)?;
            require_no_more_arguments(&mut arguments)?;

            let trusted = load_repository_contract().map_err(|error| error.to_string())?;
            let report =
                measure_identity_timing_child(&trusted, compiler_instance_id, graph_profile, n)
                    .map_err(|error| error.to_string())?;
            let json = serde_json::to_string(&report)
                .map_err(|error| format!("无法序列化冷实例子进程计时结果：{error}"))?;
            println!("{json}");
            Ok(())
        }
        "smoke-identity-fresh-process-pilot" => {
            let pilot_id = next_utf8_argument(&mut arguments, "pilot-id")?;
            let graph_profile =
                parse_graph_profile(&next_utf8_argument(&mut arguments, "graph-profile")?)?;
            let n = parse_positive_n(&next_utf8_argument(&mut arguments, "N")?)?;
            require_no_more_arguments(&mut arguments)?;

            let executable = std::env::current_exe()
                .map_err(|error| format!("无法定位当前研究执行器：{error}"))?;
            let report = run_identity_fresh_process_pilot(&executable, &pilot_id, graph_profile, n)
                .map_err(|error| error.to_string())?;
            let json = serde_json::to_string_pretty(&report)
                .map_err(|error| format!("无法序列化冷实例试运行结果：{error}"))?;
            println!("{json}");
            Ok(())
        }
        _ => Err(usage()),
    }
}

fn next_utf8_argument(
    arguments: &mut impl Iterator<Item = OsString>,
    name: &str,
) -> Result<String, String> {
    arguments
        .next()
        .ok_or_else(usage)?
        .into_string()
        .map_err(|_| format!("参数 {name} 必须是有效 UTF-8"))
}

fn require_no_more_arguments(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    if arguments.next().is_some() {
        return Err(usage());
    }
    Ok(())
}

fn parse_graph_profile(value: &str) -> Result<GraphProfileId, String> {
    match value {
        "deep-chain-v1" => Ok(GraphProfileId::DeepChain),
        "wide-star-v1" => Ok(GraphProfileId::WideStar),
        "shared-fanin-dag-v1" => Ok(GraphProfileId::SharedFaninDag),
        _ => Err(format!(
            "未知模块图配置档 {value:?}；应为 deep-chain-v1、wide-star-v1 或 shared-fanin-dag-v1"
        )),
    }
}

fn parse_positive_n(value: &str) -> Result<u32, String> {
    let n = value
        .parse::<u32>()
        .map_err(|error| format!("N 必须是正 u32 整数：{error}"))?;
    if n == 0 {
        return Err("N 必须大于零".to_owned());
    }
    Ok(n)
}

fn write_known_vector(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let json = serde_json::to_string_pretty(value)
        .map_err(|error| format!("无法序列化已知向量 {}：{error}", path.display()))?;
    std::fs::write(path, format!("{json}\n"))
        .map_err(|error| format!("无法写入已知向量 {}：{error}", path.display()))
}

fn usage() -> String {
    [
        "用法：issue-308-compiler-budget-calibration-research <命令>",
        "  verify-contract",
        "  print-module-graph-known-vectors",
        "  print-identity-known-vectors",
        "  write-known-vectors",
        "  verify-identity-oracle",
        "  identity-timing-child <compiler-instance-id> <graph-profile> <N>",
        "  smoke-identity-fresh-process-pilot <pilot-id> <graph-profile> <N>",
    ]
    .join("\n")
}
