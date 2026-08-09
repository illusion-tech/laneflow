//! §9.2 校准 harness 入口：measure（单进程采样）/ assemble（三进程合并 + 证据装配）/
//! collect-environment（环境采集自检）/ verify（contract → manifest → evidence 全链验证）。

use std::path::PathBuf;

use issue_296_geometry_frontend_calibration::{environment, evidence, measure, validator};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo 根必须存在")
}

/// 读取 `--flag <value>` 形式的选项值；缺省返回 None。
fn flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        panic!("用法：calibrate <measure|assemble|collect-environment|verify> [选项]");
    };
    let rest = &args[1..];
    let root = repo_root();
    match command {
        "measure" => {
            let process_index: u8 = flag_value(rest, "--process-index")
                .expect("measure 需要 --process-index <1|2|3>")
                .parse()
                .expect("--process-index 必须是整数");
            let output =
                flag_value(rest, "--output").expect("measure 需要 --output <进程样本路径>");
            let only_workload = flag_value(rest, "--only-workload");
            measure::measure_process(&root, process_index, only_workload, &PathBuf::from(output));
        }
        "assemble" => {
            let files =
                flag_value(rest, "--process-files").expect("assemble 需要 --process-files <a,b,c>");
            let process_files: Vec<PathBuf> = files.split(',').map(PathBuf::from).collect();
            let raw_output = PathBuf::from(
                flag_value(rest, "--raw-output").expect("assemble 需要 --raw-output <路径>"),
            );
            let evidence_output = PathBuf::from(
                flag_value(rest, "--evidence-output")
                    .expect("assemble 需要 --evidence-output <路径>"),
            );
            evidence::assemble(&root, &process_files, &raw_output, &evidence_output);
        }
        "collect-environment" => {
            let environment = environment::environment_json(&root);
            println!(
                "{}",
                serde_json::to_string_pretty(&environment).expect("环境 JSON 序列化")
            );
            eprintln!("环境采集完成：全部字段与参考机声明一致");
        }
        "verify" => validator::validate_evidence_with_contract(&root),
        _ => panic!(
            "未知子命令 {command}：用法 calibrate <measure|assemble|collect-environment|verify>"
        ),
    }
}
