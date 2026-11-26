use super::{
    add_road_editing_approach, compile_road_editing_revision, conflict_profile, install_fixture,
    road_editing_line,
};
use laneflow_compiler::road_editing::*;
use laneflow_compiler::{CompileProfile, GeometryAccuracyProfile, GeometryDirectionProfile};
use laneflow_runtime::{
    RouteError, RouteHandoffTrial, RouteHandoffTrialError, RouteRegisterInput, WorldConfig,
};
use laneflow_static_contract::{AccessEffect, CanonicalFrameOrdinal, RoadSectionOrdinal};
use laneflow_static_network::{
    BoundaryOwner, SharedNetworkRevision,
    vehicle::{
        AxleDimensions, BodyEnvelope, GeometryWorkLimits, HandoffOutcome, HandoffQuery,
        SteeringLimits,
    },
};
use std::sync::Arc;

fn revision() -> Arc<SharedNetworkRevision> {
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
    builder
        .add_alignment(
            RoadAlignmentInput::try_new(
                "road",
                CanonicalFrameReference::local("frame-main").unwrap(),
                road_editing_line((0., -10.), (0., 40.)),
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
    add(D::RoadCorridor(
        RoadCorridorInput::try_new(
            "road",
            RoadAlignmentReference::try_new("road").unwrap(),
            0.,
            RoadEditingStationEnd::AlignmentEnd,
            section.clone(),
            lanes[0].clone(),
            vec![RoadEditingCorridorElement::RoadSection(section.clone())],
        )
        .unwrap()
        .with_cross_section(CrossSectionCoverage::Complete, vec![]),
    ));
    add(D::RoadSection(
        RoadSectionInput::try_new(
            "section",
            "motorLane",
            lanes.to_vec(),
            RoadCorridorReference::local("road").unwrap(),
        )
        .unwrap()
        .with_boundary_extents(vec![BoundaryExtentInput {
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
        }]),
    ));
    for (lane, keys, cuts) in [
        ("source", &["s0", "s1", "s2", "s3"][..], vec![10., 22., 26.]),
        ("target", &["t0", "t1", "t2"][..], vec![10., 28.]),
    ] {
        for (i, k) in keys.iter().enumerate() {
            add(D::LaneEdge(
                LaneEdgeInput::try_new(
                    *k,
                    10.,
                    keys.get(i + 1)
                        .copied()
                        .or_else(|| (*k == "t2").then_some("return"))
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
                RoadEditingLaneDirection::Forward,
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
    ] {
        add(D::ParticipantClass(
            ParticipantClassInput::try_new(key).unwrap(),
        ));
        add(D::VehicleProfile(
            VehicleProfileInput::try_new(
                key,
                reference,
                IidmVehicleProfileInput::try_new(4.15, 13., 2., 1.2, 2., 3., 7.).unwrap(),
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
    // 目标链末端经普通返程回到源链首端，使 A→B 的两次出现之间全为合法后继。
    let p = |x, z| RoadEditingPoint3::try_new(x, 0., z).unwrap();
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
    compile_road_editing_revision(builder.finish().unwrap())
}

#[test]
fn trial_locates_only_the_named_occurrence_without_committing_a_route() {
    let rev = revision();
    let mut world = install_fixture(Arc::clone(&rev), WorldConfig::new(2, 4, 64, 32, 100)).unwrap();
    let relations = world.traffic().relations();
    let section = [0, 1]
        .into_iter()
        .map(RoadSectionOrdinal::from_raw)
        .find(|&s| relations.section_lanes(s).unwrap().len() == 2)
        .unwrap();
    let corridor = relations.section_corridor(section).unwrap();
    let lanes = relations.section_lanes(section).unwrap();
    let s = relations.authoring_edge_chain(lanes[0]).unwrap();
    let t = relations.authoring_edge_chain(lanes[1]).unwrap();
    let route = RouteRegisterInput::new(vec![s[0], s[1], s[2], t[1], t[2]]);
    let mut trial = RouteHandoffTrial {
        route: &route,
        source_index: 2,
        owner: BoundaryOwner::Section(section),
        left_index: 0,
        stations_nm: [24_000_000_000, 26_000_000_000],
        profile: conflict_profile(&rev, "car"),
        query: HandoffQuery {
            revision: rev.network_revision(),
            corridor,
            frame: CanonicalFrameOrdinal::from_raw(0),
            station_window_nm: [0, 50_000_000_000],
            rear_stations_nm: [10_000_000_000, 40_000_000_000],
            axles: AxleDimensions {
                wheelbase_m: 2.60,
                half_track_m: 0.80,
                reference_height_m: 0.55,
            },
        },
        steering: SteeringLimits {
            max_angle_rad: 0.6,
            max_rate_rad_s: 0.6,
            speed_m_s: 1.,
        },
        body: BodyEnvelope {
            rear_overhang_m: 0.75,
            front_from_rear_m: 3.40,
            half_width_m: 0.95,
            heights_m: [0., 1.50],
        },
        limits: GeometryWorkLimits::STANDARD,
    };
    let cursor = world.command_cursor();
    let allowed = world.preview_route_handoff(&trial).unwrap();
    assert_eq!(allowed.source_index, 2);
    assert_eq!(allowed.side_access_work, 2);
    let HandoffOutcome::Located { addresses, .. } = allowed.report.outcome else {
        panic!("{allowed:?}")
    };
    assert_eq!(addresses.map(|a| a.edge), [s[2], t[1]]);
    assert!(addresses[0].progress_mm[0] <= 3_195 && addresses[0].progress_mm[1] >= 3_195);
    assert!(addresses[1].progress_mm[0] <= 15_195 && addresses[1].progress_mm[1] >= 15_195);

    let sweep_used = allowed.report.work + allowed.report.geometry.unwrap().sweep.work;
    trial.limits.sweep = sweep_used + 1;
    assert_eq!(
        world.preview_route_handoff(&trial).unwrap_err(),
        RouteHandoffTrialError::SideAccessUnproven { route_edge: t[1] }
    );
    trial.limits.sweep += 1;
    assert_eq!(
        world.preview_route_handoff(&trial).unwrap().report,
        allowed.report
    );
    trial.limits = GeometryWorkLimits::STANDARD;
    trial.stations_nm[1] = 25_000_000_000;
    let outside = world.preview_route_handoff(&trial).unwrap();
    assert!(matches!(
        outside.report.outcome,
        HandoffOutcome::OutsideWindow { .. }
    ));
    assert_eq!(outside.side_access_work, 0);
    trial.profile = conflict_profile(&rev, "denied");
    assert_eq!(
        world.preview_route_handoff(&trial).unwrap_err(),
        RouteHandoffTrialError::AccessDenied
    );
    // 车头已交给 t1，源车道在 26 m 又切到 s3。这条源侧边不在路线出现序列中。
    trial.stations_nm[1] = 26_000_000_000;
    trial.profile = conflict_profile(&rev, "swept");
    assert!(!route.edges().contains(&s[3]));
    assert_eq!(
        world.preview_route_handoff(&trial).unwrap_err(),
        RouteHandoffTrialError::SideAccessUnproven { route_edge: s[2] }
    );
    trial.profile = conflict_profile(&rev, "car");
    let turn = world.traffic().successors(t[2]).unwrap()[0];
    let second_visit =
        RouteRegisterInput::new(vec![t[1], t[2], turn, s[0], s[1], s[2], t[1], t[2]]);
    trial.route = &second_visit;
    trial.source_index = 5;
    let revisited = world.preview_route_handoff(&trial).unwrap();
    assert_eq!(revisited.source_index, 5);
    assert_eq!(revisited.report, allowed.report);
    trial.body.rear_overhang_m += 0.001;
    assert_eq!(
        world.preview_route_handoff(&trial).unwrap_err(),
        RouteHandoffTrialError::LengthMismatch
    );
    trial.body.rear_overhang_m = 0.75;

    let repeated = RouteRegisterInput::new(vec![s[2], t[1], t[2], turn, s[0], s[1], s[2], t[1]]);
    trial.route = &repeated;
    for i in [0, 6] {
        trial.source_index = i;
        assert_eq!(
            world.preview_route_handoff(&trial).unwrap_err(),
            RouteHandoffTrialError::Route(RouteError::Disconnected)
        );
    }
    trial.source_index = u32::MAX;
    assert_eq!(
        world.preview_route_handoff(&trial).unwrap_err(),
        RouteHandoffTrialError::OccurrenceOutOfRange
    );
    assert_eq!(world.register_route(route), Err(RouteError::Disconnected));
    assert_eq!(world.command_cursor(), cursor);
    assert_eq!(world.live_routes().count(), 0);
}
