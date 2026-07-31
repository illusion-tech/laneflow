mod support;

use issue_308_compiler_budget_calibration_research::{
    ScalableWorkloadId, attribution_binary_descriptor, load_repository_contract,
    measure_identity_attribution_child, measure_scalable_attribution_ladder_child,
    wait_for_parent_start_signal,
};
use std::str::FromStr;

const USAGE: &str = "用法：issue-308-compiler-budget-calibration-attribution <describe-role|run|run-ladder>\n  run <compiler-instance-id> <graph-profile> <N> <controlled-allocation-hard-ceiling-bytes>\n  run-ladder <compiler-instance-id> <workload-id> <graph-profile> <N> <controlled-allocation-hard-ceiling-bytes>";

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
            support::print_json(&attribution_binary_descriptor(), "归因角色描述")
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
            let hard_ceiling_bytes = support::parse_positive_u64(
                &support::next_utf8_argument(
                    &mut arguments,
                    "controlled-allocation-hard-ceiling-bytes",
                    USAGE,
                )?,
                "controlled-allocation-hard-ceiling-bytes",
            )?;
            support::require_no_more_arguments(&mut arguments, USAGE)?;

            wait_for_parent_start_signal().map_err(|error| error.to_string())?;
            let trusted = load_repository_contract().map_err(|error| error.to_string())?;
            let report = measure_identity_attribution_child(
                &trusted,
                compiler_instance_id,
                graph_profile,
                n,
                hard_ceiling_bytes,
            )
            .map_err(|error| error.to_string())?;
            support::print_json(&report, "归因角色结果")
        }
        "run-ladder" => {
            let compiler_instance_id =
                support::next_utf8_argument(&mut arguments, "compiler-instance-id", USAGE)?;
            let workload_id = ScalableWorkloadId::from_str(&support::next_utf8_argument(
                &mut arguments,
                "workload-id",
                USAGE,
            )?)
            .map_err(|error| error.to_string())?;
            let graph_profile = support::parse_graph_profile(&support::next_utf8_argument(
                &mut arguments,
                "graph-profile",
                USAGE,
            )?)?;
            let n = support::parse_positive_u32(
                &support::next_utf8_argument(&mut arguments, "N", USAGE)?,
                "N",
            )?;
            let hard_ceiling_bytes = support::parse_positive_u64(
                &support::next_utf8_argument(
                    &mut arguments,
                    "controlled-allocation-hard-ceiling-bytes",
                    USAGE,
                )?,
                "controlled-allocation-hard-ceiling-bytes",
            )?;
            support::require_no_more_arguments(&mut arguments, USAGE)?;

            wait_for_parent_start_signal().map_err(|error| error.to_string())?;
            let trusted = load_repository_contract().map_err(|error| error.to_string())?;
            let report = measure_scalable_attribution_ladder_child(
                &trusted,
                compiler_instance_id,
                workload_id,
                graph_profile,
                n,
                hard_ceiling_bytes,
            )
            .map_err(|error| error.to_string())?;
            support::print_json(&report, "归因角色正式阶梯结果")
        }
        _ => Err(USAGE.to_owned()),
    }
}
