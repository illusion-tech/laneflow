//! CLI 入口：`summarize`（声明式 manifest → 统一五轴汇总回执）与
//! `check-transitions`（LF-SYNTH 输入包 #1271 拓扑核查）。
//!
//! 用法见 `usage()`；长期说明在仓库 `tools/laneflow-gate-summary/README.md`。

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use laneflow_gate_summary::evidence::PathContext;
use laneflow_gate_summary::l1check;
use laneflow_gate_summary::receipt::Mode;
use laneflow_gate_summary::summary;
use laneflow_gate_summary::{Error, invalid};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn usage() -> ! {
    eprintln!(
        "用法：
  laneflow-gate-summary summarize --manifest <toml> --output <新目录> [--repo-root <目录>] [--evidence-root <目录>] [--mode trial|formal] [--require-complete]
  laneflow-gate-summary check-transitions --bundle <LF-SYNTH 输入包目录> --output <新 json 文件>

summarize：消费声明式 manifest（lf-unified-gate-manifest-1），产出 receipt.md + receipt.json。
  manifest 内仓库相对路径按 --repo-root（默认当前目录）解析；<evidence-root>/ 前缀
  路径按 --evidence-root 解析。--require-complete 拒绝未完成项（#305 正式汇总用）。
check-transitions：#1271 拓扑核查，记录格式 lf-l1-topology-check-1。
"
    );
    std::process::exit(2);
}

fn run(args: &[String]) -> Result<String, Error> {
    let Some(command) = args.first() else {
        usage();
    };
    let rest = &args[1..];
    match command.as_str() {
        "summarize" => summarize(rest),
        "check-transitions" => check_transitions(rest),
        "help" | "--help" | "-h" => usage(),
        other => Err(invalid(format!("未知子命令：{other}"))),
    }
}

fn flag(rest: &[String], name: &str) -> Option<String> {
    rest.iter()
        .position(|arg| arg == name)
        .and_then(|index| rest.get(index + 1).cloned())
}

fn require(rest: &[String], name: &str) -> String {
    flag(rest, name).unwrap_or_else(|| {
        eprintln!("error: 缺少必选参数 {name}");
        usage();
    })
}

fn summarize(rest: &[String]) -> Result<String, Error> {
    let manifest = PathBuf::from(require(rest, "--manifest"));
    let output = PathBuf::from(require(rest, "--output"));
    let repo_root = PathBuf::from(flag(rest, "--repo-root").unwrap_or_else(|| ".".to_owned()));
    let evidence_root = flag(rest, "--evidence-root").map(PathBuf::from);
    let mode = match flag(rest, "--mode").as_deref() {
        None | Some("trial") => Mode::Trial,
        Some("formal") => Mode::Formal,
        Some(other) => return Err(invalid(format!("未知模式：{other}（可选 trial|formal）"))),
    };
    let require_complete = rest.contains(&"--require-complete".to_owned());
    let ctx = PathContext {
        repo_root,
        evidence_root,
    };
    let receipt = summary::run(&manifest, &ctx, &output, mode, require_complete)?;
    let pending = receipt.pending_items.len();
    Ok(format!(
        "汇总完成（mode={}）：{}（试跑结果不作为 v0.10 结论）；未完成项 {} 组；回执写入 {}",
        mode.name(),
        if pending == 0 {
            "无未完成项"
        } else {
            "含未完成项"
        },
        pending,
        output.display()
    ))
}

fn check_transitions(rest: &[String]) -> Result<String, Error> {
    let bundle = PathBuf::from(require(rest, "--bundle"));
    let output = PathBuf::from(require(rest, "--output"));
    if output.exists() {
        return Err(invalid(format!("输出文件必须不存在：{}", output.display())));
    }
    let input = l1check::load_bundle(&bundle)?;
    let report = l1check::check(&input)?;
    let bytes = serde_json::to_vec_pretty(&report)?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output, bytes)?;
    Ok(format!(
        "L1 拓扑核查完成：identity={}，verdict={}，记录写入 {}",
        input.identity,
        report.verdict,
        output.display()
    ))
}
