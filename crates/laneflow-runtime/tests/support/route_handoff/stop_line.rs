use super::*;
use laneflow_compiler::{GateInterpretation, GateProhibition, RegulationIdentity};
use laneflow_runtime::{TrafficWorld, deterministic_state_digest};
use laneflow_static_contract::LaneEdgeOrdinal;
use laneflow_static_network::{SharedNetworkRevision, vehicle::HandoffLaneSide};

#[derive(Clone, Copy)]
pub(super) enum SourceStop {
    OnOccurrence,
    OnContinuation,
    Upstream,
}

impl SourceStop {
    pub(super) fn add_to(self, builder: &mut RoadEditingSourceModuleBuilder) {
        use RoadEditingDeclaration as D;
        let (entry, exit) = match self {
            Self::OnOccurrence => ("s2", "s3"),
            Self::OnContinuation => ("s3", "s4"),
            Self::Upstream => ("s1", "s2"),
        };
        let edge = |key| LaneEdgeReference::local(key).unwrap();
        let junction = JunctionReference::local("junction").unwrap();
        let movement =
            MovementReference::owner_scoped(vec!["junction".into()], "straight").unwrap();
        let path =
            ManeuverPathReference::owner_scoped(vec!["junction".into(), "straight".into()], "path")
                .unwrap();
        let gate = ManeuverGateReference::owner_scoped(
            vec!["junction".into(), "straight".into(), "path".into()],
            "entry",
        )
        .unwrap();
        let rule = PolicyGateRuleInput::try_new(
            "entry",
            gate,
            None,
            GateInterpretation::Uncontrolled,
            GateProhibition::None,
            vec![],
        )
        .unwrap();
        for declaration in [
            D::Junction(
                JunctionInput::try_new("junction", vec![edge(entry), edge(exit)], vec![]).unwrap(),
            ),
            D::Movement(MovementInput::try_new("straight", junction, entry, exit).unwrap()),
            D::ManeuverPath(
                ManeuverPathInput::try_new("path", movement, edge(entry), vec![], edge(exit))
                    .unwrap(),
            ),
            D::StopLine(StopLineInput::try_new("stop", edge(entry)).unwrap()),
            D::ManeuverGate(
                ManeuverGateInput::try_new(
                    "entry",
                    path,
                    0,
                    StopLineReference::local("stop").unwrap(),
                    RoadEditingSignalControl::None,
                )
                .unwrap(),
            ),
            D::RightOfWayPolicySet(
                RightOfWayPolicySetInput::try_new(
                    "conflict-policy",
                    RegulationIdentity::try_new("engineering", "fixture-1")
                        .unwrap()
                        .with_source("repository:runtime-fixture-1")
                        .unwrap(),
                    vec![],
                    vec![],
                    vec![],
                    vec![rule],
                )
                .unwrap(),
            ),
        ] {
            builder.add_declaration(declaration).unwrap();
        }
    }
}

pub(super) fn stop_world(stop: SourceStop, direction: RoadEditingLaneDirection) -> TrafficWorld {
    let revision = compile_road_editing_revision(road_module(0., None, direction, Some(stop)));
    let config = WorldConfig::try_new(WorldConfigInput {
        vehicle_capacity: 4,
        route_capacity: 4,
        route_edge_occurrence_capacity: 64,
        route_conflict_occurrence_capacity: 32,
        fixed_delta_time_ms: 100,
    })
    .expect("有效的停止线测试配置");
    install_fixture(revision, config).unwrap()
}

pub(super) fn road_lanes(
    revision: &SharedNetworkRevision,
) -> (RoadSectionOrdinal, &[LaneEdgeOrdinal], &[LaneEdgeOrdinal]) {
    let rel = revision.traffic().relations();
    let section = [0, 1]
        .into_iter()
        .map(RoadSectionOrdinal::from_raw)
        .find(|&section| rel.section_lanes(section).unwrap().len() == 2)
        .unwrap();
    let lanes = rel.section_lanes(section).unwrap();
    (
        section,
        rel.authoring_edge_chain(lanes[0]).unwrap(),
        rel.authoring_edge_chain(lanes[1]).unwrap(),
    )
}

pub(super) fn car_trial(
    revision: &SharedNetworkRevision,
    rear_stations_nm: [u64; 2],
) -> RouteHandoffTrial {
    RouteHandoffTrial {
        profile: revision
            .identity()
            .stable_id(conflict_profile(revision, "car"))
            .unwrap(),
        station_window_nm: [0, 50_000_000_000],
        rear_stations_nm,
        speed_m_s: 1.,
        limits: GeometryWorkLimits::STANDARD,
    }
}

#[test]
fn source_stop_line_allows_a_cleared_handoff_and_rejects_an_overlapping_sweep() {
    for (stop, stop_index) in [
        (SourceStop::OnOccurrence, 2),
        (SourceStop::OnContinuation, 3),
    ] {
        let mut world = stop_world(stop, RoadEditingLaneDirection::Forward);
        let revision = world.revision();
        let rel = revision.traffic().relations();
        let (section, src, dst) = road_lanes(&revision);
        let line = rel.stop_line_for_edge(src[stop_index]).unwrap();
        assert_eq!(rel.stop_line(line).unwrap().gates().len(), 1);
        let state = |world: &TrafficWorld| {
            (
                world.command_cursor(),
                world.event_cursor(),
                world.observation_state_sequence(),
                deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap(),
            )
        };
        let before = state(&world);
        assert_eq!(
            world.register_route(RouteRegisterInput::new(src[..=stop_index].to_vec())),
            Err(RouteError::ManeuverMismatch),
        );

        // 后一次将同一条 30 m 横移向前移 12 m，源侧扫过外包覆盖 35 m 停止线。
        for (start, target_index) in [(4, 1), (16, 2)] {
            let mut edges = src[..=2].to_vec();
            edges.extend_from_slice(&dst[target_index..]);
            assert_eq!(edges.contains(&src[stop_index]), stop_index == 2);
            let binding = world
                .bind_route_handoff(
                    &RouteRegisterInput::new(edges),
                    RouteHandoffOccurrence {
                        revision: revision.network_revision(),
                        source_index: 2,
                        owner: BoundaryOwner::Section(section),
                        left_index: 0,
                        stations_nm: [start + 14, start + 16].map(|m| m * 1_000_000_000),
                    },
                )
                .unwrap();
            let trial = car_trial(&revision, [start, start + 30].map(|m| m * 1_000_000_000));
            let result = world.preview_route_handoff(&binding, &trial);
            if start == 4 {
                let preview = result.unwrap();
                assert!(matches!(
                    preview.report.outcome,
                    HandoffOutcome::Located { .. }
                ));
                let end = preview
                    .sample_after_travel(0, 30_000_000, 5_000_000, trial.limits)
                    .unwrap()
                    .sample
                    .unwrap()
                    .footprint;
                assert!(end.sides[0].is_none());
                assert_eq!(end.front_side, HandoffLaneSide::Target);
                assert!(end.front.stations_nm()[0] > 35_000_000_000);
            } else {
                assert!(matches!(
                    result.unwrap_err(),
                    RouteHandoffTrialError::StopLineUnproven { edge, reason: None, work }
                        if edge == src[stop_index] && work > 0
                ));
            }
        }
        assert_eq!(world.live_routes().count(), 0);
        assert_eq!(state(&world), before);
    }
}

#[test]
fn upstream_stop_line_uses_only_the_maneuver_present_at_this_occurrence() {
    let world = stop_world(SourceStop::Upstream, RoadEditingLaneDirection::Forward);
    let revision = world.revision();
    let (section, src, dst) = road_lanes(&revision);
    let trial = car_trial(&revision, [10_000_000_000, 40_000_000_000]);
    for (start_index, revisit) in [(0, false), (2, false), (2, true)] {
        let mut edges = src[start_index..=2].to_vec();
        edges.extend_from_slice(&dst[1..]);
        if revisit {
            // 在路线后段回到源车道，第二次经过时选定的同名机动不能替代首次交接。
            edges.push(revision.traffic().successors(dst[2]).unwrap()[0]);
            edges.extend_from_slice(src);
        }
        let binding = world
            .bind_route_handoff(
                &RouteRegisterInput::new(edges),
                RouteHandoffOccurrence {
                    revision: revision.network_revision(),
                    source_index: u32::try_from(2 - start_index).unwrap(),
                    owner: BoundaryOwner::Section(section),
                    left_index: 0,
                    stations_nm: [24_000_000_000, 26_000_000_000],
                },
            )
            .unwrap();
        let result = world.preview_route_handoff(&binding, &trial);
        if start_index == 0 {
            // 候选经过 22 m 线，来源路线在此处选定完整的 s1 → s2 机动。
            assert!(matches!(
                result.unwrap().report.outcome,
                HandoffOutcome::Located { .. }
            ));
        } else {
            // 从 s2 开始登记，虽然试算几何完全相同，上游 s1 的门没有路线机动。
            assert!(matches!(result.unwrap_err(),
                RouteHandoffTrialError::StopLineUnproven { edge, reason: None, work }
                    if edge == src[1] && work > 0));
        }
    }

    // 同一 22 m 线仍在查询窗口内。这次车辆从 25 m 起步，整个源侧扫过都已经在下游。
    let binding = world
        .bind_route_handoff(
            &RouteRegisterInput::new(vec![src[3], dst[2]]),
            RouteHandoffOccurrence {
                revision: revision.network_revision(),
                source_index: 0,
                owner: BoundaryOwner::Section(section),
                left_index: 0,
                stations_nm: [34_000_000_000, 38_000_000_000],
            },
        )
        .unwrap();
    let trial = car_trial(&revision, [25_000_000_000, 45_000_000_000]);
    assert!(matches!(
        world
            .preview_route_handoff(&binding, &trial)
            .unwrap()
            .report
            .outcome,
        HandoffOutcome::Located { .. }
    ));
}
