mod commit_messages;
mod external_review;
mod gate_evidence;
mod markdown_tables;
mod schema_publication;

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
            let path = args
                .get(1)
                .ok_or("缺少 commit message 文件路径，例如: cargo +1.96.0 run --locked -p xtask -- check-commit-message-file .git/COMMIT_EDITMSG")?;
            commit_messages::check_commit_message_file(path)
        }
        Some("check-gate-evidence") => gate_evidence::check_gate_evidence(&args[1..]),
        Some("check-gate-evidence-target") => gate_evidence::check_gate_evidence_target(&args[1..]),
        Some("check-g3-shadow-success-eligibility") => {
            gate_evidence::check_g3_shadow_success_eligibility(&args[1..])
        }
        Some("check-g3-evidence-marker") => gate_evidence::check_g3_evidence_marker(&args[1..]),
        Some("resolve-g3-evidence-shadow-targets") => {
            gate_evidence::resolve_g3_evidence_shadow_targets(&args[1..])
        }
        Some("resolve-g3-evidence-shadow-issue-event-targets") => {
            gate_evidence::resolve_g3_evidence_shadow_issue_event_targets(&args[1..])
        }
        Some("check-external-review") => external_review::run(&args[1..]),
        Some("publish-external-review-check") => {
            external_review::run_publish_check(&args[1..])
        }
        Some("format-md-tables") => markdown_tables::run(&args[1..]),
        Some("check-schema-publication-contract") => schema_publication::check_schema_publication_contract(),
        Some("build-schema-publication") => match args.as_slice() {
            [_, output_directory] => schema_publication::build_schema_publication(output_directory),
            _ => Err(
                "用法：cargo +1.96.0 run --locked -p xtask -- build-schema-publication <output-directory>"
                    .to_string(),
            ),
        },
        Some(command) => Err(format!("未知 xtask 命令: {command}")),
        None => Err(
            "缺少 xtask 命令。可用命令: check-commit-messages, check-commit-message-file, check-gate-evidence, check-gate-evidence-target, check-g3-shadow-success-eligibility, check-g3-evidence-marker, resolve-g3-evidence-shadow-targets, resolve-g3-evidence-shadow-issue-event-targets, check-external-review, publish-external-review-check, format-md-tables, check-schema-publication-contract, build-schema-publication"
                .to_string(),
        ),
    }
}
