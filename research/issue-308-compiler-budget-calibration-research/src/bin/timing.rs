mod support;

use issue_308_compiler_budget_calibration_research::{
    load_repository_contract, measure_identity_timing_child, timing_binary_descriptor,
    wait_for_parent_start_signal,
};

const USAGE: &str = "用法：issue-308-compiler-budget-calibration-timing <describe-role|run>\n  run <compiler-instance-id> <graph-profile> <N>";

fn main() {
    support::main_with(run);
}

fn run() -> Result<(), String> {
    let mut arguments = std::env::args_os();
    let _binary = arguments.next();
    let command = support::next_utf8_argument(&mut arguments, "command", USAGE)?;
    match command.as_str() {
        "describe-role" => {
            support::require_no_more_arguments(&mut arguments, USAGE)?;
            support::print_json(&timing_binary_descriptor(), "计时角色描述")
        }
        "run" => {
            let compiler_instance_id =
                support::next_utf8_argument(&mut arguments, "compiler-instance-id", USAGE)?;
            let graph_profile = support::parse_graph_profile(&support::next_utf8_argument(
                &mut arguments,
                "graph-profile",
                USAGE,
            )?)?;
            let n = support::parse_positive_u32(
                &support::next_utf8_argument(&mut arguments, "N", USAGE)?,
                "N",
            )?;
            support::require_no_more_arguments(&mut arguments, USAGE)?;

            wait_for_parent_start_signal().map_err(|error| error.to_string())?;
            let trusted = load_repository_contract().map_err(|error| error.to_string())?;
            let report =
                measure_identity_timing_child(&trusted, compiler_instance_id, graph_profile, n)
                    .map_err(|error| error.to_string())?;
            support::print_json(&report, "计时角色结果")
        }
        _ => Err(USAGE.to_owned()),
    }
}
