mod support;

use issue_308_compiler_budget_calibration_research::{
    CandidateKeyDomain, CandidateRegistry, ConstantHashRole, LimitPairPlan, LimitPairSide,
    LimitQualificationScale, ScalableWorkloadId, TIMING_BINARY_ID, load_repository_contract,
    measure_candidate_kernel_child, measure_candidate_pipeline_child,
    measure_constant_hash_observation, measure_duplicate_owner_child,
    measure_identity_timing_child, measure_limit_side_child, measure_scalable_timing_child,
    measure_scalable_timing_ladder_child, timing_binary_descriptor, wait_for_parent_start_signal,
};
use std::str::FromStr;

const USAGE: &str = "用法：issue-308-compiler-budget-calibration-timing <describe-role|run|run-ladder|run-identity-smoke|run-candidate-kernel|run-candidate-pipeline|run-constant-hash|run-limit-side|run-duplicate-owner>\n  run <compiler-instance-id> <workload-id> <graph-profile> <N> <controlled-allocation-hard-ceiling-bytes>\n  run-ladder <compiler-instance-id> <workload-id> <graph-profile> <N> <controlled-allocation-hard-ceiling-bytes>\n  run-identity-smoke <compiler-instance-id> <graph-profile> <N>\n  run-candidate-kernel <candidate-id> <key-domain> <item-count>\n  run-candidate-pipeline <compiler-instance-id> <candidate-id> <key-domain> <workload-id> <graph-profile> <N> <controlled-allocation-hard-ceiling-bytes>\n  run-constant-hash <candidate-id> <input-variant-id> <repeat>\n  run-limit-side <compiler-instance-id> <scale-json> <pair-json> <at-bound|plus-one>\n  run-duplicate-owner <compiler-instance-id> <scale-json>";

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
        "run" | "run-ladder" => {
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
            let controlled_allocation_hard_ceiling_bytes = support::parse_positive_u64(
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
                let report = measure_scalable_timing_ladder_child(
                    &trusted,
                    compiler_instance_id,
                    workload_id,
                    graph_profile,
                    n,
                    controlled_allocation_hard_ceiling_bytes,
                )
                .map_err(|error| error.to_string())?;
                support::print_json(&report, "计时角色正式阶梯结果")
            } else {
                let report = measure_scalable_timing_child(
                    &trusted,
                    compiler_instance_id,
                    workload_id,
                    graph_profile,
                    n,
                    controlled_allocation_hard_ceiling_bytes,
                )
                .map_err(|error| error.to_string())?;
                support::print_json(&report, "计时角色结果")
            }
        }
        "run-identity-smoke" => {
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
            support::print_json(&report, "标识工作负载计时冒烟结果")
        }
        "run-candidate-kernel" => {
            let candidate_id = support::next_utf8_argument(&mut arguments, "candidate-id", USAGE)?;
            let key_domain = CandidateKeyDomain::from_str(&support::next_utf8_argument(
                &mut arguments,
                "key-domain",
                USAGE,
            )?)
            .map_err(|error| error.to_string())?;
            let item_count = support::parse_positive_u32(
                &support::next_utf8_argument(&mut arguments, "item-count", USAGE)?,
                "item-count",
            )?;
            support::require_no_more_arguments(&mut arguments, USAGE)?;

            wait_for_parent_start_signal().map_err(|error| error.to_string())?;
            let trusted = load_repository_contract().map_err(|error| error.to_string())?;
            let registry = CandidateRegistry::from_trusted_contract(&trusted)
                .map_err(|error| error.to_string())?;
            if !registry
                .candidates_for(key_domain)
                .any(|candidate| candidate.id == candidate_id)
            {
                return Err(format!(
                    "候选 {candidate_id} 未注册到键域 {}",
                    key_domain.as_str()
                ));
            }
            let report = measure_candidate_kernel_child(&candidate_id, key_domain, item_count)
                .map_err(|error| error.to_string())?;
            support::print_json(&report, "候选机制计时结果")
        }
        "run-candidate-pipeline" => {
            let compiler_instance_id =
                support::next_utf8_argument(&mut arguments, "compiler-instance-id", USAGE)?;
            let candidate_id = support::next_utf8_argument(&mut arguments, "candidate-id", USAGE)?;
            let key_domain = CandidateKeyDomain::from_str(&support::next_utf8_argument(
                &mut arguments,
                "key-domain",
                USAGE,
            )?)
            .map_err(|error| error.to_string())?;
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
            let controlled_allocation_hard_ceiling_bytes = support::parse_positive_u64(
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
            let report = measure_candidate_pipeline_child(
                &trusted,
                compiler_instance_id,
                &candidate_id,
                key_domain,
                workload_id,
                graph_profile,
                n,
                controlled_allocation_hard_ceiling_bytes,
            )
            .map_err(|error| error.to_string())?;
            support::print_json(&report, "完整候选研究管线计时结果")
        }
        "run-constant-hash" => {
            let candidate_id = support::next_utf8_argument(&mut arguments, "candidate-id", USAGE)?;
            let input_variant_id =
                support::next_utf8_argument(&mut arguments, "input-variant-id", USAGE)?;
            let repeat = support::parse_u32(
                &support::next_utf8_argument(&mut arguments, "repeat", USAGE)?,
                "repeat",
            )?;
            support::require_no_more_arguments(&mut arguments, USAGE)?;

            wait_for_parent_start_signal().map_err(|error| error.to_string())?;
            let trusted = load_repository_contract().map_err(|error| error.to_string())?;
            let report = measure_constant_hash_observation(
                &trusted,
                &candidate_id,
                ConstantHashRole::CandidateUnderTest,
                &input_variant_id,
                repeat,
                TIMING_BINARY_ID,
            )
            .map_err(|error| error.to_string())?;
            support::print_json(&report, "恒定哈希候选正确性结果")
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
                TIMING_BINARY_ID,
                compiler_instance_id,
                &scale,
                &pair,
                side,
            )
            .map_err(|error| error.to_string())?;
            support::print_json(&report, "计时角色限制侧结果")
        }
        "run-duplicate-owner" => {
            let compiler_instance_id =
                support::next_utf8_argument(&mut arguments, "compiler-instance-id", USAGE)?;
            let scale = support::parse_json::<LimitQualificationScale>(
                &support::next_utf8_argument(&mut arguments, "scale-json", USAGE)?,
                "scale-json",
            )?;
            support::require_no_more_arguments(&mut arguments, USAGE)?;

            wait_for_parent_start_signal().map_err(|error| error.to_string())?;
            let trusted = load_repository_contract().map_err(|error| error.to_string())?;
            let report = measure_duplicate_owner_child(&trusted, compiler_instance_id, &scale)
                .map_err(|error| error.to_string())?;
            support::print_json(&report, "计时角色重复所有者语义失败结果")
        }
        _ => Err(USAGE.to_owned()),
    }
}
