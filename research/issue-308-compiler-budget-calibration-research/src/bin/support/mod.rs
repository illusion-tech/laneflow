use issue_308_compiler_budget_calibration_research::GraphProfileId;
use serde::{Deserialize, Serialize};
use std::ffi::OsString;

pub fn next_utf8_argument(
    arguments: &mut impl Iterator<Item = OsString>,
    name: &str,
    usage: &str,
) -> Result<String, String> {
    arguments
        .next()
        .ok_or_else(|| usage.to_owned())?
        .into_string()
        .map_err(|_| format!("参数 {name} 必须是有效 UTF-8"))
}

pub fn require_no_more_arguments(
    arguments: &mut impl Iterator<Item = OsString>,
    usage: &str,
) -> Result<(), String> {
    if arguments.next().is_some() {
        return Err(usage.to_owned());
    }
    Ok(())
}

pub fn parse_graph_profile(value: &str) -> Result<GraphProfileId, String> {
    match value {
        "deep-chain-v1" => Ok(GraphProfileId::DeepChain),
        "wide-star-v1" => Ok(GraphProfileId::WideStar),
        "shared-fanin-dag-v1" => Ok(GraphProfileId::SharedFaninDag),
        _ => Err(format!(
            "未知模块图配置档 {value:?}；应为 deep-chain-v1、wide-star-v1 或 shared-fanin-dag-v1"
        )),
    }
}

pub fn parse_positive_u32(value: &str, name: &str) -> Result<u32, String> {
    let number = value
        .parse::<u32>()
        .map_err(|error| format!("{name} 必须是正 u32 整数：{error}"))?;
    if number == 0 {
        return Err(format!("{name} 必须大于零"));
    }
    Ok(number)
}

#[allow(dead_code)]
pub fn parse_u32(value: &str, name: &str) -> Result<u32, String> {
    value
        .parse::<u32>()
        .map_err(|error| format!("{name} 必须是 u32 整数：{error}"))
}

#[allow(dead_code)]
pub fn parse_positive_u64(value: &str, name: &str) -> Result<u64, String> {
    let number = value
        .parse::<u64>()
        .map_err(|error| format!("{name} 必须是正 u64 整数：{error}"))?;
    if number == 0 {
        return Err(format!("{name} 必须大于零"));
    }
    Ok(number)
}

#[allow(dead_code)]
pub fn parse_json<'de, T: Deserialize<'de>>(value: &'de str, name: &str) -> Result<T, String> {
    serde_json::from_str(value).map_err(|error| format!("参数 {name} 不是合法 JSON：{error}"))
}

pub fn print_json(value: &impl Serialize, context: &str) -> Result<(), String> {
    let json =
        serde_json::to_string(value).map_err(|error| format!("无法序列化{context}：{error}"))?;
    println!("{json}");
    Ok(())
}

pub fn main_with(run: impl FnOnce() -> Result<(), String>) {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
