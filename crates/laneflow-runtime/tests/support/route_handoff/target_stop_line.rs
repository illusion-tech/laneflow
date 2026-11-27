use super::stop_line::{SourceStop, car_trial, road_lanes, stop_world};
use super::*;
use laneflow_runtime::deterministic_state_digest;
use laneflow_static_network::vehicle::HandoffLaneSide;

fn stations(direction: RoadEditingLaneDirection, distances_m: [u64; 2]) -> [u64; 2] {
    distances_m.map(|m| match direction {
        RoadEditingLaneDirection::Forward => m * 1_000,
        RoadEditingLaneDirection::Backward => (50 - m) * 1_000,
    })
}

#[test]
fn a_body_point_reaches_the_target_stop_before_the_front_bumper_handoff() {
    let gated = stop_world(SourceStop::Upstream, RoadEditingLaneDirection::Forward);
    let control =
        install_fixture(compile_road_editing_revision(module(0.)), gated.config()).unwrap();
    for (world, has_stop) in [(&control, false), (&gated, true)] {
        let revision = world.revision();
        let (section, target, source) = road_lanes(&revision);
        let binding = world
            .bind_route_handoff(
                &RouteRegisterInput::new(vec![source[0], source[1], target[2], target[3]]),
                RouteHandoffOccurrence {
                    revision: revision.network_revision(),
                    source_index: 1,
                    owner: BoundaryOwner::Section(section),
                    left_index: 0,
                    stations_mm: [24_000, 26_000],
                },
            )
            .unwrap();
        let trial = car_trial(&revision, [10_000, 40_000]);
        let result = world.preview_route_handoff(&binding, &trial);
        if has_stop {
            assert!(matches!(result.unwrap_err(),
                RouteHandoffTrialError::StopLineUnproven { edge, reason: None, .. } if edge == target[1]));
        } else {
            let preview = result.unwrap();
            // 直接取相同路面和车型的实体点见证。两个姿态的前保险杠中点都在源侧。
            let witness = |travel| {
                let footprint = preview
                    .sample_after_travel(0, travel, TRAVEL_TOLERANCE_MM, trial.limits)
                    .unwrap()
                    .sample
                    .unwrap()
                    .footprint;
                assert_eq!(footprint.front_side, HandoffLaneSide::Source);
                footprint.sides[1].as_ref().unwrap().witness.stations_mm()
            };
            assert!(witness(8_500)[1] < 22_000);
            assert!(witness(10_000)[0] > 22_000);
        }
    }
}

#[test]
fn the_target_stop_requires_the_physical_successor_at_this_occurrence() {
    let direction = RoadEditingLaneDirection::Forward;
    let world = stop_world(SourceStop::WithTurn, direction);
    let revision = world.revision();
    let (section, target, source) = road_lanes(&revision);
    let traffic = revision.traffic();
    let line = traffic.relations().stop_line_for_edge(target[1]).unwrap();
    assert_eq!(
        traffic.relations().stop_line(line).unwrap().gates().len(),
        2
    );
    let turn = traffic
        .maneuvers()
        .transition_candidates(target[1])
        .unwrap()
        .iter()
        .map(|c| c.successor())
        .find(|&edge| edge != target[2])
        .unwrap();
    let occurrence = RouteHandoffOccurrence {
        revision: revision.network_revision(),
        source_index: 1,
        owner: BoundaryOwner::Section(section),
        left_index: 0,
        stations_mm: [18_000, 20_000],
    };
    // 相同出现和停止截面。选定转向门不能放行扫入直行续边的车身。
    let trial = car_trial(&revision, [4_000, 34_000]);
    for (successor, allowed) in [(target[2], true), (turn, false)] {
        let route = RouteRegisterInput::new(vec![source[0], source[1], target[1], successor]);
        let bound = world.bind_route_handoff(&route, occurrence).unwrap();
        let result = world.preview_route_handoff(&bound, &trial);
        if allowed {
            assert!(matches!(
                result.unwrap().report.outcome,
                HandoffOutcome::Located { .. }
            ));
        } else {
            assert!(
                matches!(result.unwrap_err(), RouteHandoffTrialError::StopLineUnproven { edge, reason: None, .. } if edge == target[1])
            );
        }
    }

    // 第一次交接跳过目标车道的 s1。以后回到该车道时才选定 s1→s2 的直行门。
    let return_edge = traffic.successors(source[2]).unwrap()[0];
    let route = RouteRegisterInput::new(vec![
        source[0],
        source[1],
        target[2],
        target[3],
        source[2],
        return_edge,
        target[0],
        target[1],
        target[2],
        target[3],
    ]);
    let first = RouteHandoffOccurrence {
        stations_mm: [24_000, 26_000],
        ..occurrence
    };
    let second = RouteHandoffOccurrence {
        source_index: 3,
        stations_mm: [34_000, 36_000],
        ..occurrence
    };
    let bound = world.bind_route_handoffs(&route, &[first, second]).unwrap();
    let first_trial = car_trial(&revision, [10_000, 40_000]);
    assert!(
        matches!(world.preview_route_handoff_at(&bound, 1, &first_trial).unwrap_err(),
        RouteHandoffTrialError::StopLineUnproven { edge, reason: None, .. } if edge == target[1])
    );
}

#[test]
fn target_stop_requires_a_selected_maneuver_or_whole_body_separation_in_both_directions() {
    for direction in [
        RoadEditingLaneDirection::Forward,
        RoadEditingLaneDirection::Backward,
    ] {
        let world = stop_world(SourceStop::Upstream, direction);
        let revision = world.revision();
        // 交换横移方向。夹具中 s1 末端的线现在位于目标道的行进 22 m 处。
        let (section, target, source) = road_lanes(&revision);
        let line = revision
            .traffic()
            .relations()
            .stop_line_for_edge(target[1])
            .unwrap();
        assert_eq!(
            revision
                .traffic()
                .relations()
                .stop_line(line)
                .unwrap()
                .gates()
                .len(),
            1
        );
        let state = || {
            (
                world.command_cursor(),
                world.event_cursor(),
                world.observation_state_sequence(),
                deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap(),
            )
        };
        let before = state();
        for (rear, seam, source_index, target_index, clears) in [
            ([10, 40], [24, 26], 1, 2, false),
            ([4, 34], [18, 20], 1, 1, true),
            ([25, 45], [34, 38], 2, 3, true),
        ] {
            let mut edges = source[..=source_index].to_vec();
            edges.extend_from_slice(&target[target_index..]);
            let mut window = stations(direction, seam);
            window.sort_unstable();
            let occurrence = RouteHandoffOccurrence {
                revision: revision.network_revision(),
                source_index: u32::try_from(source_index).unwrap(),
                owner: BoundaryOwner::Section(section),
                left_index: 0,
                stations_mm: window,
            };
            let binding = world
                .bind_route_handoff(&RouteRegisterInput::new(edges), occurrence)
                .unwrap();
            let trial = car_trial(&revision, stations(direction, rear));
            let result = world.preview_route_handoff(&binding, &trial);
            if clears {
                assert!(matches!(
                    result.unwrap().report.outcome,
                    HandoffOutcome::Located { .. }
                ));
            } else {
                assert!(matches!(result.unwrap_err(),
                    RouteHandoffTrialError::StopLineUnproven { edge, reason: None, work }
                    if edge == target[1] && work > 0));
            }
        }
        assert_eq!(state(), before);
    }
}
