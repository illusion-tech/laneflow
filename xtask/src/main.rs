mod commit_messages;
mod markdown_tables;
mod schema_codegen;
mod wire_audit;

use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("check-commit-messages") => commit_messages::check_commit_messages(args.get(1).map(String::as_str)),
        Some("check-commit-message-file") => {
            let path = args.get(1).ok_or(
                "缺少 commit message 文件路径，例如: cargo +1.98.0 run --locked -p xtask -- check-commit-message-file .git/COMMIT_EDITMSG",
            )?;
            commit_messages::check_commit_message_file(path)
        }
        Some("format-md-tables") => markdown_tables::run(&args[1..]),
        Some("check-road-editing-codegen") => schema_codegen::run(&schema_codegen::ROAD_EDITING, &args[1..]),
        Some("check-runtime-snapshot-codegen") => {
            schema_codegen::run(&schema_codegen::RUNTIME_SNAPSHOT, &args[1..])
        }
        Some("check-wire-audit") => wire_audit::run(),
        Some(command) => Err(format!("未知 xtask 命令: {command}")),
        None => Err(
            "缺少 xtask 命令。可用命令: check-commit-messages, check-commit-message-file, format-md-tables, check-road-editing-codegen, check-runtime-snapshot-codegen, check-wire-audit"
                .to_string(),
        ),
    }
}
