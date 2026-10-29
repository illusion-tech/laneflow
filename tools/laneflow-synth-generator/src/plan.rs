use serde_json::{Value, json};

use crate::{Case, EDGE_MM, Identity};

fn command(operation: &str, rank: u32, occurrence: u32, progress: u32, speed: u32) -> Value {
    json!({"operation":operation, "logical_rank":rank, "route_rank":rank / 25,
        "edge_occurrence":occurrence, "progress_mm":progress, "speed_mm_per_s":speed})
}

fn reset(ranks: &[u32], occurrence: u32, progress: u32, speed: u32) -> Vec<Value> {
    ranks
        .iter()
        .flat_map(|&rank| {
            [
                command("despawn", rank, 0, 0, 0),
                command("spawn", rank, occurrence, progress, speed),
            ]
        })
        .collect()
}

pub(crate) fn input_plan(identity: &Identity) -> Value {
    let ranks = |local_rank| {
        (0..identity.cells())
            .map(|cell| cell * 100 + local_rank)
            .collect::<Vec<_>>()
    };
    let warm = identity.warm_up_ticks();
    let mut scheduled = Vec::new();
    let mut rules = vec![json!({
        "kind":"invariants", "individuals":identity.scale,
        "logical_identity":"rank_survives_handle_replacement",
        "command_failure":"invalidate_round", "undeclared_mutation":"invalidate_round",
        "ordinary_completion":"invalidate_round", "event_order":"runtime_return_order",
        "completion_observation":"compare_pre_post_status_by_logical_rank",
        "allowed_persistent_blocked":false,
        "actual_receipts":"record_actual_tick_separately_from_plan"
    })];
    match identity.case {
        Case::W1 | Case::W2 => {
            rules.push(json!({"kind":"command_free", "active":if identity.case == Case::W1 { identity.scale * 3 / 4 } else { identity.scale },
                "parked":if identity.case == Case::W1 { identity.scale / 4 } else { 0 },
                "parking_bindings":"unchanged", "completed":0}));
        }
        Case::W3 => {
            let parking = ranks(0);
            let lifecycle = ranks(81);
            scheduled.push(json!({"boundary":0,"commands":parking.iter().map(|&rank|command("reserve_parking",rank,1,5_000,0)).collect::<Vec<_>>()}));
            scheduled.push(json!({"boundary":warm,"commands":reset(&lifecycle,1,EDGE_MM,0)}));
            rules.push(json!({
                "kind":"parking_arrival_barrier", "ranks":parking,
                "expected_arrivals_per_rank":1, "warm_up_arrival":"invalidate_round",
                "missing_or_duplicate_arrival":"invalidate_round", "collect_order":"runtime_return_order",
                "commit_trigger":"next_boundary_after_all_exact_pairs_arrive",
                "commit_order":"cell_ascending", "commit_operation":"park_vehicle",
                "commit_outcome":"Committed", "minimum_observation_steps_remaining":2,
                "leave_trigger":"next_boundary_after_one_successful_step",
                "leave_operation":"leave_parking", "leave_outcome":"ParkingLeaveRecord",
                "leave_order":"cell_ascending", "edge_occurrence":1,"progress_mm":5_000,"speed_mm_per_s":0,
                "parked_tick_active":identity.scale * 24 / 100,
                "parked_tick_parked":identity.scale * 76 / 100,
                "restored_active":identity.scale / 4,"restored_parked":identity.scale * 3 / 4,
                "max_reservations_per_rank":1,"cancel_or_rebind":"invalidate_round"
            }));
            rules.push(json!({
                "kind":"completed_replacement", "ranks":lifecycle,
                "observe":"Active_to_Completed", "observe_order":"logical_rank_ascending",
                "trigger":"next_boundary_after_completion", "operation":"atomic_replace",
                "edge_occurrence":0,"progress_mm":188_000,"speed_mm_per_s":13_900,
                "blocked":"retry_same_frozen_input_in_same_order_at_later_boundary",
                "persistent_blocked":"invalidate_round", "expected_completions_per_rank":1
            }));
        }
        Case::W4 => {
            let lifecycle = ranks(87);
            let parking = ranks(62);
            let transition = ranks(61);
            scheduled.push(json!({"boundary":warm-1,"commands":reset(&lifecycle,1,EDGE_MM,0)}));
            let mut burst: Vec<_> = lifecycle
                .iter()
                .map(|&rank| command("atomic_replace", rank, 0, 356_000, 13_900))
                .collect();
            burst.extend(
                parking
                    .iter()
                    .map(|&rank| command("leave_parking", rank, 1, 95_000, 0)),
            );
            scheduled.push(json!({"boundary":warm,"commands":burst}));
            scheduled
                .push(json!({"boundary":warm+1,"commands":reset(&transition,0,5_129_900,13_900)}));
            rules.push(json!({
                "kind":"synchronized_burst", "lifecycle_ranks":lifecycle,
                "parking_ranks":parking,"transition_ranks":transition,
                "b_minus_1_completion":"required_once_per_lifecycle_rank",
                "b0_signal":"all_controllers_enter_A_green",
                "b0_transition":"forbidden", "b1_transition":"entry_to_exit_once_per_transition_rank",
                "b1_commands_require":"successful_B0", "extra_step_or_presentation_commit":"forbidden",
                "post_b0_active":identity.scale * 76 / 100,"post_b0_parked":identity.scale * 24 / 100,
                "stop":"immediately_after_B1_commit_events_and_presentation", "blocked":"invalidate_round"
            }));
        }
    }
    if matches!(identity.case, Case::W3 | Case::W4) {
        rules.push(json!({
            "kind":"failed_step_retry", "build":"optimized_test_only",
            "trigger":if identity.case == Case::W3 {"step_after_first_atomic_replacements"} else {"B0_step"},
            "failpoint":"StepFailpoint::AfterTransitions", "error":"StepError::ParkingObservationAllocFailed",
            "capture":"post_command_pre_step_state_and_canonical_snapshot",
            "digest":"deterministic_state_digest", "failed_step_preserves":["state","published_events","command_cursor"],
            "retry":"same_step_without_repeating_commands",
            "oracle":"independent_pre_command_world_same_batch_same_post_command_state_then_same_step",
            "instrumented_timing":"diagnostic_only"
        }));
    }
    let boundary_order = if identity.case == Case::W4 {
        vec![
            "scheduled_commands_in_array_order",
            "fixed_step",
            "consume_ordered_events",
            "enqueue_next_boundary_plans",
        ]
    } else {
        vec![
            "parking_commits_by_cell",
            "parking_leaves_by_cell",
            "lifecycle_commands_by_cell",
            "fixed_step",
            "consume_ordered_events",
            "enqueue_next_boundary_plans",
        ]
    };
    json!({"format":"lf-synth-input-plan-1", "identity":identity,
        "warm_up_ticks":warm,
        "observation_ticks":if identity.case == Case::W4 { 2 } else { 2 * warm },
        "boundary_order":boundary_order,
        "scheduled":scheduled,"rules":rules})
}
