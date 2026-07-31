use issue_308_compiler_budget_calibration_research::{
    CONTRACT_DESCRIPTOR_BYTE_LENGTH, GraphProfileId, PilotBudgetRequest, load_repository_contract,
    parse_formal_protocol_arguments, recompute_pilot_budget, run_formal_protocol,
    run_identity_fresh_process_pilot, runner_binary_descriptor,
};
use std::ffi::OsString;
use std::path::PathBuf;

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
        "describe-role" => {
            require_no_more_arguments(&mut arguments)?;
            let json = serde_json::to_string(&runner_binary_descriptor())
                .map_err(|error| format!("无法序列化执行器角色描述：{error}"))?;
            println!("{json}");
            Ok(())
        }
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
        "smoke-identity-fresh-process-pilot" => {
            let pilot_id = next_utf8_argument(&mut arguments, "pilot-id")?;
            let graph_profile =
                parse_graph_profile(&next_utf8_argument(&mut arguments, "graph-profile")?)?;
            let n = parse_positive_n(&next_utf8_argument(&mut arguments, "N")?)?;
            let explicit_timing_binary = arguments.next().map(PathBuf::from);
            require_no_more_arguments(&mut arguments)?;

            let trusted = load_repository_contract().map_err(|error| error.to_string())?;
            let timing_binary = explicit_timing_binary
                .map(Ok)
                .unwrap_or_else(resolve_sibling_timing_binary)?;
            let report = run_identity_fresh_process_pilot(
                &trusted,
                &timing_binary,
                &pilot_id,
                graph_profile,
                n,
                None,
            )
            .map_err(|error| error.to_string())?;
            let json = serde_json::to_string_pretty(&report)
                .map_err(|error| format!("无法序列化冷实例试运行结果：{error}"))?;
            println!("{json}");
            Ok(())
        }
        "run" => {
            let request =
                parse_formal_protocol_arguments(arguments).map_err(|error| error.to_string())?;
            let outcome = run_formal_protocol(&request).map_err(|error| error.to_string())?;
            let json = serde_json::to_string_pretty(&outcome)
                .map_err(|error| format!("无法序列化正式协议执行结果：{error}"))?;
            println!("{json}");
            Ok(())
        }
        "recompute-pilot-budget" => {
            let input_path = PathBuf::from(next_utf8_argument(&mut arguments, "input")?);
            let json_output_path =
                PathBuf::from(next_utf8_argument(&mut arguments, "json-output")?);
            let markdown_output_path =
                PathBuf::from(next_utf8_argument(&mut arguments, "markdown-output")?);
            require_no_more_arguments(&mut arguments)?;
            let outcome = recompute_pilot_budget(&PilotBudgetRequest {
                input_path,
                json_output_path,
                markdown_output_path,
            })
            .map_err(|error| error.to_string())?;
            let json = serde_json::to_string_pretty(&outcome)
                .map_err(|error| format!("无法序列化预算重算结果：{error}"))?;
            println!("{json}");
            Ok(())
        }
        _ => Err(usage()),
    }
}

fn resolve_sibling_timing_binary() -> Result<PathBuf, String> {
    let runner =
        std::env::current_exe().map_err(|error| format!("无法定位当前研究执行器：{error}"))?;
    let directory = runner
        .parent()
        .ok_or_else(|| format!("研究执行器没有父目录：{}", runner.display()))?;
    let timing = directory.join(format!(
        "issue-308-compiler-budget-calibration-timing{}",
        std::env::consts::EXE_SUFFIX
    ));
    if !timing.is_file() {
        return Err(format!(
            "未找到同目录计时角色二进制 {}；请先使用 cargo build --bins 构建全部角色，或把精确路径作为最后一个参数传入",
            timing.display()
        ));
    }
    Ok(timing)
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

fn usage() -> String {
    [
        "用法：issue-308-compiler-budget-calibration-research <命令>",
        "  describe-role",
        "  verify-contract",
        "  smoke-identity-fresh-process-pilot <pilot-id> <graph-profile> <N> [timing-binary-path]",
        "  run --protocol compiler-calibration-v1 --output <formal-execution-checkpoint-path>",
        "  recompute-pilot-budget <checkpoint-path> <json-output-path> <markdown-output-path>",
    ]
    .join("\n")
}
