//! 复用无窗口示例的装配与运行，检查走廊上的一辆车确实被推进。

use laneflow_runtime::{ObservationExportMode, VehicleStatus, WorldPolicySelection};

#[path = "../examples/headless.rs"]
mod headless;

#[test]
fn checked_corridor_advances_one_vehicle_and_bad_delta_does_not_commit() {
    let report = headless::run().expect("headless example");
    let text = report.render();

    assert!(
        matches!(report.policy, WorldPolicySelection::Pinned(_)),
        "corridor catalog must pin a policy"
    );
    assert!(report.source_revision_matches, "published source revision");
    assert_eq!(
        report.network_revision,
        "2de4820d35bee0ef71374cae49036fbef75155a2a587cbd13b351d5f7655ad27"
    );
    assert_eq!(report.portal_id, "portal-main-west");
    assert_eq!(report.lane_index, 0);
    assert_eq!(report.slot_id, "slot-main-west-lane-0-000");
    assert_eq!(report.route_id, "route-main-west-near-left");
    assert_eq!(report.fixed_delta_time_ms, 16);
    assert_eq!(report.worker_count, 1);
    assert_eq!(report.route_edges, 3);
    assert_eq!(report.live_vehicles_after_rejected_route, 1);
    assert!(
        report.command_cursor_unchanged,
        "rejected route registration must not consume the command cursor"
    );
    assert_eq!(report.time_after_rejected_step_ms, 0);
    assert_eq!(report.steps, 64);
    assert_eq!(report.time_ms, 1_024);
    assert_eq!(report.tick_index, 64);
    assert!(report.advanced, "vehicle did not move");
    assert_eq!(report.status, VehicleStatus::Active);
    assert_eq!(report.edge_before, 0);
    assert_eq!(report.edge_after, 0);
    assert_eq!(report.progress_before_mm, 6_500);
    assert_eq!(report.progress_after_mm, 23_567);
    assert_eq!(report.speed_mm_s, 16_667);
    assert!(report.rejected_delta_left_session_empty);
    assert_eq!(report.observation_mode, ObservationExportMode::Full);
    assert!(report.observation_base_is_none);
    assert_eq!(report.observation_revision, report.network_revision);
    assert_eq!(report.observation_tick, report.tick_index);
    assert!(report.observation_world_matches);
    assert_eq!(report.observation_entries, 66);
    assert_eq!(report.occupied_length_mm, 4_500);
    assert_eq!(report.front_vehicle_count, 1);
    assert!(text.contains(&format!("time_ms={}", report.time_ms)));
    assert!(text.contains(&format!("network_revision={}", report.network_revision)));
    assert!(text.contains("rejected_step=DeltaMismatch"));
    assert!(text.contains("rejected_observation=FirstExportMustBeFull"));
}
