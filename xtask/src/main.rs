mod commit_messages;
mod markdown_tables;
mod road_editing_codegen;

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
                "缺少 commit message 文件路径，例如: cargo +1.96.0 run --locked -p xtask -- check-commit-message-file .git/COMMIT_EDITMSG",
            )?;
            commit_messages::check_commit_message_file(path)
        }
        Some("format-md-tables") => markdown_tables::run(&args[1..]),
        Some("check-road-editing-codegen") => road_editing_codegen::run(&args[1..]),
        Some(command) => Err(format!("未知 xtask 命令: {command}")),
        None => Err(
            "缺少 xtask 命令。可用命令: check-commit-messages, check-commit-message-file, format-md-tables, check-road-editing-codegen"
                .to_string(),
        ),
    }
}
