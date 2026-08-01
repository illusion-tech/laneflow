mod support;

use issue_308_compiler_budget_calibration_research::{
    ATTRIBUTION_BINARY_ID, CleanupFailureCase, LimitPairPlan, LimitPairSide,
    LimitQualificationScale, ScalableWorkloadId, attribution_binary_descriptor,
    load_repository_contract, measure_cleanup_child, measure_identity_attribution_child,
    measure_limit_side_child, measure_scalable_attribution_child,
    measure_scalable_attribution_ladder_child, wait_for_parent_start_signal,
};
use std::str::FromStr;

const USAGE: &str = "用法：issue-308-compiler-budget-calibration-attribution <describe-role|run|run-preflight|run-ladder|run-limit-side|run-cleanup>\n  run <compiler-instance-id> <graph-profile> <N> <controlled-allocation-hard-ceiling-bytes>\n  run-preflight <compiler-instance-id> <workload-id> <graph-profile> <N> <controlled-allocation-hard-ceiling-bytes>\n  run-ladder <compiler-instance-id> <workload-id> <graph-profile> <N> <controlled-allocation-hard-ceiling-bytes>\n  run-limit-side <compiler-instance-id> <scale-json> <pair-json> <at-bound|plus-one>\n  run-cleanup <scale-json> <case-json>";

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
        "run-preflight" | "run-ladder" => {
            let ladder = command == "run-ladder";
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
            if ladder {
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
            } else {
                let report = measure_scalable_attribution_child(
                    &trusted,
                    compiler_instance_id,
                    workload_id,
                    graph_profile,
                    n,
                    hard_ceiling_bytes,
                )
                .map_err(|error| error.to_string())?;
                support::print_json(&report, "归因角色护栏预检结果")
            }
        }
        "run-limit-side" => {
            let compiler_instance_id =
                support::next_utf8_argument(&mut arguments, "compiler-instance-id", USAGE)?;
            let scale = support::parse_json::<LimitQualificationScale>(
                &support::next_utf8_argument(&mut arguments, "scale-json", USAGE)?,
                "scale-json",
            )?;
            let pair = support::parse_json::<LimitPairPlan>(
                &support::next_utf8_argument(&mut arguments, "pair-json", USAGE)?,
                "pair-json",
            )?;
            let side = match support::next_utf8_argument(&mut arguments, "side", USAGE)?.as_str() {
                "at-bound" => LimitPairSide::AtBound,
                "plus-one" => LimitPairSide::PlusOne,
                _ => return Err(USAGE.to_owned()),
            };
            support::require_no_more_arguments(&mut arguments, USAGE)?;

            wait_for_parent_start_signal().map_err(|error| error.to_string())?;
            let trusted = load_repository_contract().map_err(|error| error.to_string())?;
            let report = measure_limit_side_child(
                &trusted,
                ATTRIBUTION_BINARY_ID,
                compiler_instance_id,
                &scale,
                &pair,
                side,
            )
            .map_err(|error| error.to_string())?;
            support::print_json(&report, "归因角色限制侧结果")
        }
        "run-cleanup" => {
            let scale = support::parse_json::<LimitQualificationScale>(
                &support::next_utf8_argument(&mut arguments, "scale-json", USAGE)?,
                "scale-json",
            )?;
            let case_id = support::parse_json::<CleanupFailureCase>(
                &support::next_utf8_argument(&mut arguments, "case-json", USAGE)?,
                "case-json",
            )?;
            support::require_no_more_arguments(&mut arguments, USAGE)?;

            wait_for_parent_start_signal().map_err(|error| error.to_string())?;
            let trusted = load_repository_contract().map_err(|error| error.to_string())?;
            let report = measure_cleanup_child(&trusted, &scale, case_id)
                .map_err(|error| error.to_string())?;
            support::print_json(&report, "归因角色清理实验结果")
        }
        _ => Err(USAGE.to_owned()),
    }
}
