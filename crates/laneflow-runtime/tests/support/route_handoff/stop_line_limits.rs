use super::stop_line::{SourceStop, car_trial, road_lanes, stop_world};
use super::*;
use laneflow_runtime::{TrafficWorld, deterministic_state_digest};
use laneflow_static_network::StationQuery;
use laneflow_static_network::vehicle::Failure;

#[test]
fn both_stop_lines_share_one_sweep_and_report_its_work_on_target_failure() {
    let direction = RoadEditingLaneDirection::Forward;
    let source_only = stop_world(SourceStop::OnOccurrence, direction);
    let both = stop_world(SourceStop::Both, direction);
    let revision = both.revision();
    let (_, source, target) = road_lanes(&revision);
    let source_line = source[2];
    let target_line = target[0];
    for (edge, station) in [(source_line, 35_000_000_000), (target_line, 10_000_000_000)] {
        assert!(
            revision
                .traffic()
                .relations()
                .stop_line_for_edge(edge)
                .is_some()
        );
        assert_eq!(
            revision.station_mapping().edge_end_station_nm(edge),
            Some(station)
        );
    }
    let preview = |world: &TrafficWorld, rear_m: [u64; 2], budget| {
        let revision = world.revision();
        let (section, source, target) = road_lanes(&revision);
        // 两个版本的物理切点相同，目标段从停止线下游 t1 开始。
        let mut edges = source[..=2].to_vec();
        edges.extend_from_slice(&target[1..]);
        let binding = world
            .bind_route_handoff(
                &RouteRegisterInput::new(edges),
                RouteHandoffOccurrence {
                    revision: revision.network_revision(),
                    source_index: 2,
                    owner: BoundaryOwner::Section(section),
                    left_index: 0,
                    stations_nm: [14_000_000_000, 25_000_000_000],
                },
            )
            .unwrap();
        let mut trial = car_trial(&revision, rear_m.map(|m| m * 1_000_000_000));
        trial.limits.sweep = budget;
        let check = world.preview_route_handoff(&binding, &trial)?;
        assert!(
            matches!(check.report.outcome, HandoffOutcome::Located { .. }),
            "{check:?}"
        );
        let before_stop =
            check.report.work + check.report.geometry.unwrap().sweep.work + check.side_access_work;
        Ok((before_stop, check.stop_line_work))
    };
    let (before_stop, once) =
        preview(&source_only, [10, 30], GeometryWorkLimits::STANDARD.sweep).unwrap();
    assert!(once > 1);
    let exact = before_stop + once;
    assert_eq!(preview(&both, [10, 30], exact), Ok((before_stop, once)));
    assert_eq!(
        preview(&both, [10, 30], exact - 1),
        Err(RouteHandoffTrialError::StopLineUnproven {
            edge: source_line,
            reason: Some(Failure::WorkLimit),
            work: once - 1,
        })
    );

    // 起点提前：源侧仍在 35 m 前清空，目标侧外包触及 10 m。
    let (before_stop, once) =
        preview(&source_only, [1, 31], GeometryWorkLimits::STANDARD.sweep).unwrap();
    assert!(once > 1);
    assert_eq!(
        preview(&both, [1, 31], before_stop + once),
        Err(RouteHandoffTrialError::StopLineUnproven {
            edge: target_line,
            reason: None,
            work: once,
        })
    );
}

#[test]
fn reverse_handoff_uses_the_original_sweep_budget_to_clear_the_source_stop_line() {
    let world = stop_world(SourceStop::OnOccurrence, RoadEditingLaneDirection::Backward);
    let revision = world.revision();
    let (section, src, dst) = road_lanes(&revision);
    assert!(
        revision
            .traffic()
            .relations()
            .stop_line_for_edge(src[2])
            .is_some()
    );
    let digest = || deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap();
    let before = digest();
    // 从高站位向低站位推进，交接点在源边 s2 内。放宽站位窗口以包含停止线。
    let bind = |start: u64, target: usize| {
        let mut edges = src[..=2].to_vec();
        edges.extend_from_slice(&dst[target..]);
        world
            .bind_route_handoff(
                &RouteRegisterInput::new(edges),
                RouteHandoffOccurrence {
                    revision: revision.network_revision(),
                    source_index: 2,
                    owner: BoundaryOwner::Section(section),
                    left_index: 0,
                    stations_nm: [start - 16, start - 14].map(|s| s * 1_000_000_000),
                },
            )
            .unwrap()
    };
    let trial = |start: u64| car_trial(&revision, [start, start - 30].map(|s| s * 1_000_000_000));
    let binding = bind(46, 1);
    let relations = revision.traffic().relations();
    let source_lane = relations.lane_edge_authoring_lane(src[2]).unwrap();
    let area = binding.boundary_at(2).unwrap();
    assert!(!area.increasing());
    // 源边末端位于递减方向的 15 m 站位，精确切点归下一源边零点。
    let StationQuery::Complete(seam) =
        revision
            .station_mapping()
            .at_station(area.corridor(), 15_000_000_000, source_lane)
    else {
        panic!("未定位反向停止线")
    };
    assert!(
        seam.addresses()
            .map(|a| (a.edge, a.progress_mm))
            .eq([(src[3], [0, 0])])
    );
    let mut clear = trial(46);
    let full = world.preview_route_handoff(&binding, &clear).unwrap();
    assert!(
        matches!(full.report.outcome, HandoffOutcome::Located { .. }),
        "{full:?}"
    );
    assert!(full.stop_line_work > 1);
    let geometry = full.report.geometry.unwrap();
    let budget =
        geometry.sweep.work + full.report.work + full.side_access_work + full.stop_line_work;
    clear.limits.sweep = budget;
    let exact = world.preview_route_handoff(&binding, &clear).unwrap();
    assert_eq!(exact.report.outcome, full.report.outcome);
    assert_eq!(exact.stop_line_work, full.stop_line_work);
    clear.limits.sweep -= 1;
    assert_eq!(
        world.preview_route_handoff(&binding, &clear).unwrap_err(),
        RouteHandoffTrialError::StopLineUnproven {
            edge: src[2],
            reason: Some(Failure::WorkLimit),
            work: full.stop_line_work - 1,
        }
    );
    // 原准入与两侧边链恰好用完预算，停止线求证无法执行第一项工作。
    clear.limits.sweep = budget - full.stop_line_work;
    assert_eq!(
        world.preview_route_handoff(&binding, &clear).unwrap_err(),
        RouteHandoffTrialError::StopLineUnproven {
            edge: src[2],
            reason: Some(Failure::WorkLimit),
            work: 0,
        }
    );
    let crossing = bind(34, 2);
    assert!(matches!(
        world.preview_route_handoff(&crossing, &trial(34)).unwrap_err(),
        RouteHandoffTrialError::StopLineUnproven { edge, reason: None, .. } if edge == src[2]
    ));
    assert_eq!(digest(), before);
}

#[test]
fn target_access_denial_precedes_source_stop_line_sweep() {
    let world = stop_world(SourceStop::OnOccurrence, RoadEditingLaneDirection::Forward);
    let revision = world.revision();
    let (section, src, dst) = road_lanes(&revision);
    // 普通目标段仅有 t1。整车投影窗口还包含 t2，而该边对 denied 车型禁行。
    let binding = world
        .bind_route_handoff(
            &RouteRegisterInput::new(vec![src[0], src[1], src[2], dst[1]]),
            RouteHandoffOccurrence {
                revision: revision.network_revision(),
                source_index: 2,
                owner: BoundaryOwner::Section(section),
                left_index: 0,
                stations_nm: [18_000_000_000, 20_000_000_000],
            },
        )
        .unwrap();
    let mut trial = car_trial(&revision, [4_000_000_000, 34_000_000_000]);
    let full = world.preview_route_handoff(&binding, &trial).unwrap();
    assert!(matches!(
        full.report.outcome,
        HandoffOutcome::Located { .. }
    ));
    assert!(full.stop_line_work > 1);
    let before_stop =
        full.report.work + full.report.geometry.unwrap().sweep.work + full.side_access_work;
    trial.limits.sweep = before_stop;
    assert!(matches!(
        world.preview_route_handoff(&binding, &trial).unwrap_err(),
        RouteHandoffTrialError::StopLineUnproven { edge, reason: Some(Failure::WorkLimit), .. }
            if edge == src[2]
    ));
    trial.profile = revision
        .identity()
        .stable_id(conflict_profile(&revision, "denied"))
        .unwrap();
    for sweep in [GeometryWorkLimits::STANDARD.sweep, before_stop] {
        trial.limits.sweep = sweep;
        assert_eq!(
            world.preview_route_handoff(&binding, &trial).unwrap_err(),
            RouteHandoffTrialError::SideAccessUnproven { route_edge: dst[1] }
        );
    }
}
