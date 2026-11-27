use super::{
    add_road_editing_approach, compile_road_editing_revision, conflict_profile, install_fixture,
    road_editing_line,
};
use laneflow_compiler::road_editing::*;
use laneflow_compiler::{
    CompileProfile, GeometryAccuracyProfile, GeometryDirectionProfile, RigidTwoAxleGeometryInput,
    RoadVehicleGeometryInput,
};
use laneflow_runtime::{
    RouteError, RouteHandoffOccurrence, RouteHandoffTrial, RouteHandoffTrialError,
    RouteRegisterInput, WorldConfig, WorldConfigInput,
};
use laneflow_static_contract::{AccessEffect, NetworkRevisionId, RoadSectionOrdinal, Sha256Digest};
use laneflow_static_network::{
    BoundaryOwner,
    vehicle::{
        GeometryWorkLimits, HandoffBodySide, HandoffOutcome, HandoffTravelTarget, SWEEP_WORK_LIMIT,
    },
};
use std::sync::Arc;

#[path = "route_handoff/approaches.rs"]
mod approaches;
#[path = "route_handoff/multi_handoff.rs"]
mod multi_handoff;
#[path = "route_handoff/rebind.rs"]
mod rebind;
#[path = "route_handoff/sample.rs"]
mod sample;
#[path = "route_handoff/stop_line.rs"]
mod stop_line;
#[path = "route_handoff/stop_line_limits.rs"]
mod stop_line_limits;
#[path = "route_handoff/traffic.rs"]
mod traffic;

#[path = "route_handoff/clearance_speed.rs"]
mod clearance_speed;

fn module(shift_x: f64) -> RoadEditingSourceModule {
    module_with_boundary(shift_x, None, RoadEditingLaneDirection::Forward)
}

fn module_with_boundary(
    shift_x: f64,
    additional: Option<BoundaryExtentInput>,
    direction: RoadEditingLaneDirection,
) -> RoadEditingSourceModule {
    road_module(shift_x, additional, direction, None)
}

fn road_module(
    shift_x: f64,
    additional: Option<BoundaryExtentInput>,
    direction: RoadEditingLaneDirection,
    source_stop: Option<stop_line::SourceStop>,
) -> RoadEditingSourceModule {
    let limits = CompileProfile::single_network_10k_stable_entities();
    let header = RoadEditingModuleHeader::try_new(
        "city/runtime-conflict",
        "runtime-conflict.lfre",
        vec![],
        RoadEditingProvenance::direct("route handoff").unwrap(),
    )
    .unwrap();
    let mut builder = RoadEditingSourceModuleBuilder::new(
        header,
        GeometryAccuracyProfile::Fine2Cm,
        GeometryDirectionProfile::Smooth1Deg,
        &limits,
    )
    .unwrap();
    let section = RoadSectionReference::owner_scoped(vec!["road".into()], "section").unwrap();
    let lanes = ["source", "target"].map(|name| {
        AuthoringLaneReference::owner_scoped(vec!["road".into(), "section".into()], name).unwrap()
    });
    let edge = |name: &str| LaneEdgeReference::local(name).unwrap();
    let class = ParticipantClassReference::local("car").unwrap();
    let denied = ParticipantClassReference::local("denied").unwrap();
    let swept = ParticipantClassReference::local("swept").unwrap();
    let fixed = ParticipantClassReference::local("fixed").unwrap();
    builder
        .add_alignment(
            RoadAlignmentInput::try_new(
                "road",
                CanonicalFrameReference::local("frame-main").unwrap(),
                road_editing_line((shift_x, -10.), (shift_x, 40.)),
            )
            .unwrap(),
        )
        .unwrap();
    use RoadEditingDeclaration as D;
    let mut add = |d| {
        builder.add_declaration(d).unwrap();
    };
    add(D::CanonicalFrame(
        CanonicalFrameInput::try_new("frame-main").unwrap(),
    ));
    let mut elements = vec![RoadEditingCorridorElement::RoadSection(section.clone())];
    let (reference_section, reference) = if direction == RoadEditingLaneDirection::Backward {
        // 另一个正向区段为反向双车道提供走廊参考线。
        let owner = RoadSectionReference::owner_scoped(vec!["road".into()], "reference").unwrap();
        let lane = AuthoringLaneReference::owner_scoped(
            vec!["road".into(), "reference".into()],
            "forward",
        )
        .unwrap();
        add(D::RoadSection(
            RoadSectionInput::try_new(
                "reference",
                "motorLane",
                vec![lane.clone()],
                RoadCorridorReference::local("road").unwrap(),
            )
            .unwrap(),
        ));
        add(D::LaneEdge(
            LaneEdgeInput::try_new("ref", 10., vec![], None).unwrap(),
        ));
        add(D::AuthoringLane(
            AuthoringLaneInput::try_new(
                "forward",
                edge("ref"),
                RoadEditingLaneDirection::Forward,
                LinearWidthProfile::try_new(3.5, 3.5).unwrap(),
                None,
                owner.clone(),
            )
            .unwrap()
            .with_wheel_support(Some(true)),
        ));
        elements.push(RoadEditingCorridorElement::RoadSection(owner.clone()));
        (owner, lane)
    } else {
        (section.clone(), lanes[0].clone())
    };
    add(D::RoadCorridor(
        RoadCorridorInput::try_new(
            "road",
            RoadAlignmentReference::try_new("road").unwrap(),
            0.,
            RoadEditingStationEnd::AlignmentEnd,
            reference_section,
            reference,
            elements,
        )
        .unwrap()
        .with_cross_section(CrossSectionCoverage::Complete, vec![]),
    ));
    let mut extents = vec![BoundaryExtentInput {
        left: BoundaryMemberInput::AuthoringLane(lanes[0].clone()),
        right: BoundaryMemberInput::AuthoringLane(lanes[1].clone()),
        kind: BoundaryExtentKind::Rule,
        local_key: "mark".into(),
        low: BoundaryStation::CorridorEndpoint,
        high: BoundaryStation::CorridorEndpoint,
        payload: Some(BoundaryPayloadInput::CrossingRule {
            directions: 3,
            effect: 1,
        }),
        canvas_selection: None,
    }];
    extents.extend(additional);
    add(D::RoadSection(
        RoadSectionInput::try_new(
            "section",
            "motorLane",
            lanes.to_vec(),
            RoadCorridorReference::local("road").unwrap(),
        )
        .unwrap()
        .with_boundary_extents(extents),
    ));
    let (source_keys, source_cuts) = match source_stop {
        None | Some(stop_line::SourceStop::Upstream) => {
            (&["s0", "s1", "s2", "s3"][..], vec![10., 22., 26.])
        }
        Some(stop_line::SourceStop::OnOccurrence) => {
            (&["s0", "s1", "s2", "s3"][..], vec![10., 14., 35.])
        }
        Some(stop_line::SourceStop::OnContinuation) => (
            &["s0", "s1", "s2", "s3", "s4"][..],
            vec![10., 14., 32., 35.],
        ),
    };
    for (lane, keys, mut cuts) in [
        ("source", source_keys, source_cuts),
        ("target", &["t0", "t1", "t2"][..], vec![10., 28.]),
    ] {
        if direction == RoadEditingLaneDirection::Backward {
            for station in &mut cuts {
                *station = 50. - *station;
            }
        }
        for (i, k) in keys.iter().enumerate() {
            add(D::LaneEdge(
                LaneEdgeInput::try_new(
                    *k,
                    10.,
                    keys.get(i + 1)
                        .copied()
                        .or_else(|| {
                            (*k == "t2" && direction == RoadEditingLaneDirection::Forward)
                                .then_some("return")
                        })
                        .map(edge)
                        .into_iter()
                        .collect(),
                    None,
                )
                .unwrap(),
            ));
        }
        add(D::AuthoringLane(
            AuthoringLaneInput::try_new(
                lane,
                edge(keys[0]),
                direction,
                LinearWidthProfile::try_new(3.5, 3.5).unwrap(),
                None,
                section.clone(),
            )
            .unwrap()
            .with_edge_chain(keys.iter().map(|k| edge(k)).collect(), cuts)
            .unwrap()
            .with_wheel_support(Some(true)),
        ));
    }
    for (key, reference) in [
        ("car", class),
        ("denied", denied.clone()),
        ("swept", swept.clone()),
        ("fixed", fixed),
    ] {
        add(D::ParticipantClass(
            ParticipantClassInput::try_new(key).unwrap(),
        ));
        add(D::VehicleProfile(
            VehicleProfileInput::try_new(
                key,
                reference,
                IidmVehicleProfileInput::try_new(4.15, 13., 2., 1.2, 2., 3., 7.).unwrap(),
                RoadVehicleGeometryInput::RigidTwoAxle(RigidTwoAxleGeometryInput {
                    wheelbase_m: 2.60,
                    track_width_m: 1.60,
                    reference_height_m: 0.55,
                    rear_overhang_m: 0.75,
                    width_m: 1.90,
                    bottom_height_m: 0.,
                    top_height_m: 1.50,
                    max_steering_angle_rad: if key == "fixed" { 0. } else { 0.6 },
                    max_steering_rate_rad_s: 0.6,
                }),
            )
            .unwrap(),
        ));
    }
    for (key, k, class) in [
        ("closed-suffix", "t2", denied),
        ("closed-source-tail", "s3", swept),
    ] {
        add(D::AccessRule(
            AccessRuleInput::try_new(
                key,
                RoadEditingAccessTarget::LaneEdge(edge(k)),
                AccessEffect::Deny,
                vec![class],
                0,
            )
            .unwrap(),
        ));
    }
    if let Some(stop) = source_stop {
        stop.add_to(&mut builder);
    }
    if direction == RoadEditingLaneDirection::Backward {
        return builder.finish().unwrap();
    }
    // 目标链末端经普通返程回到源链首端，使 A→B 的两次出现之间全为合法后继。
    let p = |x, z| RoadEditingPoint3::try_new(x + shift_x, 0., z).unwrap();
    let loop_back = RoadEditingCurveProgram::try_new(
        p(-3.5, 40.),
        vec![
            RoadEditingCurveSegment::cubic_bezier(p(-3.5, 90.), p(80., 90.), p(80., 40.)),
            RoadEditingCurveSegment::line(p(80., -10.)),
            RoadEditingCurveSegment::cubic_bezier(p(80., -60.), p(0., -60.), p(0., -10.)),
        ],
    )
    .unwrap();
    add_road_editing_approach(&mut builder, "return", loop_back, vec![edge("s0")]);
    builder.finish().unwrap()
}

#[test]
fn trial_locates_only_the_named_occurrence_without_committing_a_route() {
    let rev = compile_road_editing_revision(module(0.));
    let mut world = install_fixture(
        Arc::clone(&rev),
        WorldConfig::try_new(WorldConfigInput {
            vehicle_capacity: 2,
            route_capacity: 4,
            route_edge_occurrence_capacity: 64,
            route_conflict_occurrence_capacity: 32,
            fixed_delta_time_ms: 100,
        })
        .expect("有效的交通配置"),
    )
    .unwrap();
    let relations = world.traffic().relations();
    let section = [0, 1]
        .into_iter()
        .map(RoadSectionOrdinal::from_raw)
        .find(|&s| relations.section_lanes(s).unwrap().len() == 2)
        .unwrap();
    let lanes = relations.section_lanes(section).unwrap();
    let s = relations.authoring_edge_chain(lanes[0]).unwrap();
    let t = relations.authoring_edge_chain(lanes[1]).unwrap();
    let route = RouteRegisterInput::new(vec![s[0], s[1], s[2], t[1], t[2]]);
    let occurrence = RouteHandoffOccurrence {
        revision: rev.network_revision(),
        source_index: 2,
        owner: BoundaryOwner::Section(section),
        left_index: 0,
        stations_nm: [24_000_000_000, 26_000_000_000],
    };
    let binding = world.bind_route_handoff(&route, occurrence).unwrap();
    let profile = |key| {
        rev.identity()
            .stable_id(conflict_profile(&rev, key))
            .unwrap()
    };
    let mut trial = RouteHandoffTrial {
        profile: profile("car"),
        station_window_nm: [0, 50_000_000_000],
        rear_stations_nm: [10_000_000_000, 40_000_000_000],
        speed_m_s: 1.,
        limits: GeometryWorkLimits::STANDARD,
    };
    let cursor = world.command_cursor();
    let allowed = world.preview_route_handoff(&binding, &trial).unwrap();
    assert_eq!(allowed.source_index, 2);
    assert_eq!(allowed.side_access_work, 2);
    let HandoffOutcome::Located { addresses, .. } = allowed.report.outcome else {
        panic!("{allowed:?}")
    };
    assert_eq!(addresses.map(|a| a.edge), [s[2], t[1]]);
    assert!(addresses[0].progress_mm[0] <= 3_195 && addresses[0].progress_mm[1] >= 3_195);
    assert!(addresses[1].progress_mm[0] <= 15_195 && addresses[1].progress_mm[1] >= 15_195);

    // 同一条 30 m 五次横移的独立弧长积分。车头交接时，车身仍跨两侧。
    let travel = allowed
        .travel(
            HandoffTravelTarget::RearStationNm(40_000_000_000),
            5_000_000,
            SWEEP_WORK_LIMIT,
        )
        .unwrap();
    let [low, high] = travel.bounds.unwrap().distance_nm;
    assert!((low..=high).contains(&30_289_144_330));
    for (rear, side) in [
        (10_000_000_000, HandoffBodySide::SourceOnly),
        (21_861_190_000, HandoffBodySide::Straddling),
        (30_000_000_000, HandoffBodySide::Straddling),
        (30_500_000_000, HandoffBodySide::TargetOnly),
    ] {
        assert_eq!(
            allowed.body_side([rear; 2], SWEEP_WORK_LIMIT).unwrap().side,
            Ok(side)
        );
    }

    let sweep_used = allowed.report.work + allowed.report.geometry.unwrap().sweep.work;
    trial.limits.sweep = sweep_used + 1;
    assert_eq!(
        world.preview_route_handoff(&binding, &trial).unwrap_err(),
        RouteHandoffTrialError::SideAccessUnproven { route_edge: t[1] }
    );
    trial.limits.sweep += 1;
    assert_eq!(
        world
            .preview_route_handoff(&binding, &trial)
            .unwrap()
            .report,
        allowed.report
    );
    trial.limits = GeometryWorkLimits::STANDARD;
    let narrow = world
        .bind_route_handoff(
            &route,
            RouteHandoffOccurrence {
                stations_nm: [24_000_000_000, 25_000_000_000],
                ..occurrence
            },
        )
        .unwrap();
    let outside = world.preview_route_handoff(&narrow, &trial).unwrap();
    assert!(matches!(
        outside.report.outcome,
        HandoffOutcome::OutsideWindow { .. }
    ));
    assert_eq!(outside.side_access_work, 0);
    assert!(
        outside
            .travel(
                HandoffTravelTarget::DistanceNm(0),
                5_000_000,
                SWEEP_WORK_LIMIT
            )
            .is_none()
    );
    assert!(
        outside
            .body_side([10_000_000_000; 2], SWEEP_WORK_LIMIT)
            .is_none()
    );
    trial.profile = profile("denied");
    assert_eq!(
        world.preview_route_handoff(&narrow, &trial).unwrap_err(),
        RouteHandoffTrialError::AccessDenied
    );
    // 车头已交给 t1，源车道在 26 m 又切到 s3。这条源侧边不在路线出现序列中。
    trial.profile = profile("swept");
    assert!(!route.edges().contains(&s[3]));
    assert_eq!(
        world.preview_route_handoff(&binding, &trial).unwrap_err(),
        RouteHandoffTrialError::SideAccessUnproven { route_edge: s[2] }
    );
    trial.profile = profile("car");
    let turn = world.traffic().successors(t[2]).unwrap()[0];
    let second_visit =
        RouteRegisterInput::new(vec![t[1], t[2], turn, s[0], s[1], s[2], t[1], t[2]]);
    let second_binding = world
        .bind_route_handoff(
            &second_visit,
            RouteHandoffOccurrence {
                source_index: 5,
                ..occurrence
            },
        )
        .unwrap();
    let revisited = world
        .preview_route_handoff(&second_binding, &trial)
        .unwrap();
    assert_eq!(revisited.source_index, 5);
    assert_eq!(revisited.report, allowed.report);
    trial.profile = profile("fixed");
    let rigid = world
        .preview_route_handoff(&second_binding, &trial)
        .unwrap();
    assert!(matches!(
        rigid.report.outcome,
        HandoffOutcome::GeometryRejected
    ));
    assert!(!rigid.report.geometry.unwrap().feasible());

    let repeated = RouteRegisterInput::new(vec![s[2], t[1], t[2], turn, s[0], s[1], s[2], t[1]]);
    for i in [0, 6] {
        assert_eq!(
            world
                .bind_route_handoff(
                    &repeated,
                    RouteHandoffOccurrence {
                        source_index: i,
                        ..occurrence
                    }
                )
                .unwrap_err(),
            RouteHandoffTrialError::Route(RouteError::Disconnected)
        );
    }
    assert_eq!(
        world
            .bind_route_handoff(
                &repeated,
                RouteHandoffOccurrence {
                    source_index: u32::MAX,
                    ..occurrence
                }
            )
            .unwrap_err(),
        RouteHandoffTrialError::OccurrenceOutOfRange
    );
    let revision = NetworkRevisionId::from_digest(Sha256Digest::from_bytes([0; 32]));
    assert_eq!(
        world
            .bind_route_handoff(
                &route,
                RouteHandoffOccurrence {
                    revision,
                    ..occurrence
                }
            )
            .unwrap_err(),
        RouteHandoffTrialError::RevisionMismatch
    );
    assert_eq!(world.register_route(route), Err(RouteError::Disconnected));
    assert_eq!(world.command_cursor(), cursor);
    assert_eq!(world.live_routes().count(), 0);
}
