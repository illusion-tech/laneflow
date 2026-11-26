use super::*;
use laneflow_static_contract::{AuthoringLaneOrdinal, CanonicalFrameOrdinal, RoadCorridorOrdinal};
use laneflow_static_network::vehicle::{
    AxleDimensions, BodyEnvelope, CorridorTrace, Failure, GeometryWorkLimits, PoseBounds,
    RearAxleLine, RearAxleTrace, SteeringConstraint, SteeringLimits, SweepOutcome,
    TRACK_WORK_LIMIT, TrackOutcome, TrackQuery, Wheel,
};

const AXLES: AxleDimensions = AxleDimensions {
    wheelbase_m: 2.60,
    half_track_m: 0.80,
    reference_height_m: 0.55,
};

const LIMITS: SteeringLimits = SteeringLimits {
    max_angle_rad: 0.6,
    max_rate_rad_s: 0.020,
    speed_m_s: 19.,
};

fn crest() -> Program {
    // z=-30..30，y=-z²/2000。三次贝塞尔精确表达二次曲线，两端纵坡为 +3%、-3%。
    Program::try_new(
        p(0., -0.45, -30.),
        vec![Segment::cubic_bezier(
            p(0., 0.15, -10.),
            p(0., 0.15, 10.),
            p(0., -0.45, 30.),
        )],
    )
    .unwrap()
}

fn road(program: Program, lanes: [([f64; 2], bool); 2]) -> PortablePublicationCandidate {
    let saved = save_lane_declarations(
        program.clone(),
        &[],
        (
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Smooth1Deg,
        ),
        [RoadEditingLaneDirection::Forward; 2],
        CrossSectionCoverage::Complete,
        0,
        SectionFixture {
            lanes,
            ..SectionFixture::default()
        },
    );
    assert_eq!(read(&saved), program);
    publish(&compile_saved(&saved).unwrap(), "traffic-wheel-tracks")
}

fn query(network: &SharedNetworkRevision, trace: RearAxleLine) -> TrackQuery<'_> {
    let corridor = RoadCorridorOrdinal::try_from_usize(0).unwrap();
    let section = network
        .traffic()
        .relations()
        .corridor_reference_section(corridor)
        .unwrap();
    let lanes = network
        .traffic()
        .relations()
        .section_lanes(section)
        .unwrap();
    let mapping = network.station_mapping();
    TrackQuery {
        revision: network.network_revision(),
        corridor,
        frame: mapping.surface_spans(corridor)[0].frame,
        station_window_nm: [0, mapping.corridor_domain(corridor).unwrap().length_nm],
        traversable_lanes: lanes,
        axles: AXLES,
        trace: RearAxleTrace::Line(trace),
    }
}

fn enclosed(bounds: [f64; 2], exact: f64) {
    assert!(
        bounds[0] <= exact && exact <= bounds[1],
        "{exact} outside {bounds:?}"
    );
}

// 直接在世界抛物线上求平均高程和轴距；此对照不读发布行、求投影或用生产区间求根。
fn crest_pose(x: f64, z: f64, sin: f64, cos: f64) -> ([[f64; 3]; 4], f64) {
    let b = AXLES.half_track_m;
    let axle = |z: f64| {
        let a = -(z - b * sin).powi(2) / 2_000.;
        let c = -(z + b * sin).powi(2) / 2_000.;
        ((a + c) * 0.5, a - c)
    };
    let (rear, diff_r) = axle(z);
    let (mut lo, mut hi) = (0., AXLES.wheelbase_m);
    for _ in 0..60 {
        let d = 0.5 * (lo + hi);
        if d.hypot(axle(z + d * cos).0 - rear) < AXLES.wheelbase_m {
            lo = d;
        } else {
            hi = d;
        }
    }
    let d = (lo + hi) * 0.5;
    let (front, diff_f) = axle(z + d * cos);
    let t = [d * sin, front - rear, d * cos].map(|v| v / AXLES.wheelbase_m);
    let raw = [2. * b * cos, 0.5 * (diff_r + diff_f), -2. * b * sin];
    let dot: f64 = raw.into_iter().zip(t).map(|(a, b)| a * b).sum();
    let left = std::array::from_fn::<_, 3, _>(|i| raw[i] - dot * t[i]);
    let norm = left[0].hypot(left[1]).hypot(left[2]);
    let left = left.map(|v| v / norm);
    let up = [
        t[1] * left[2] - t[2] * left[1],
        t[2] * left[0] - t[0] * left[2],
        t[0] * left[1] - t[1] * left[0],
    ];
    (
        [[x, rear + AXLES.reference_height_m, z], t, left, up],
        (diff_f - diff_r).abs(),
    )
}

fn check_pose(pose: PoseBounds, expected: [[f64; 3]; 4]) {
    for (bounds, vector) in [pose.rear_m, pose.forward, pose.left, pose.up]
        .into_iter()
        .zip(expected)
    {
        for (interval, value) in bounds.into_iter().zip(vector) {
            enclosed(interval, value);
        }
    }
}

#[test]
fn smooth_crest_covers_the_continuous_sweep_in_both_directions() {
    let candidate = road(crest(), [([8.; 2], true), ([3.5; 2], true)]);
    let reports = [
        SpatialBuildOption::Omit,
        SpatialBuildOption::RetainAvailable,
    ]
    .map(|spatial| {
        let network = load_network(&candidate, spatial);
        let mut reports = Vec::new();
        for sin in [0.0_f64, 0.20, -0.20] {
            for direction in [-1., 1.] {
                let cos = (1. - sin * sin).sqrt() * direction;
                let sin = sin * direction;
                let z = -4. * direction;
                let line = RearAxleLine {
                    start_xz_m: [z * sin / cos, z],
                    heading_xz: [sin, cos],
                    length_nm: (8e9 / cos.abs()).ceil() as u64,
                };
                let input = query(&network, line);
                let full = network.wheel_tracks(&input, LIMITS, TRACK_WORK_LIMIT);
                let TrackOutcome::Covered(sweep) = full.outcome else {
                    panic!("{full:?}")
                };
                assert_eq!(full.covered_prefix_nm, line.length_nm);
                assert!(full.work > 0 && full.work < TRACK_WORK_LIMIT);
                reports.push(full);
                for z in [-3.5, 0., 3.5] {
                    let x = z * sin / cos;
                    let (exact, difference) = crest_pose(x, z, sin, cos);
                    if sin == 0. {
                        assert_eq!(difference, 0.);
                    } else {
                        assert!((0.000_814..0.000_816).contains(&difference));
                    }
                    check_pose(sweep, exact);
                    let at = network.wheel_tracks(
                        &TrackQuery {
                            trace: RearAxleTrace::Line(RearAxleLine {
                                start_xz_m: [x, z],
                                length_nm: 0,
                                ..line
                            }),
                            ..input
                        },
                        LIMITS,
                        TRACK_WORK_LIMIT,
                    );
                    let TrackOutcome::Covered(pose) = at.outcome else {
                        panic!("{at:?}")
                    };
                    check_pose(pose, exact);
                    assert!(pose.rear_m[1][1] - pose.rear_m[1][0] < 0.002);
                    assert!(pose.up[1][0] > 0.99);
                    reports.push(at);
                }
            }
        }
        reports
    });
    assert_eq!(reports[0], reports[1]);
}

#[test]
fn tracks_cross_a_shared_lane_edge_and_report_a_fifty_millimetre_overrun() {
    let trace = RearAxleLine {
        start_xz_m: [-4., -2.],
        heading_xz: [0., 1.],
        length_nm: 4_000_000_000,
    };
    for (support, expected) in [(true, None), (false, Some(Wheel::FrontRight))] {
        let artifact = road(crest(), [([8.; 2], true), ([3.5; 2], support)]);
        let n = load_network(&artifact, SpatialBuildOption::Omit);
        let input = query(&n, trace);
        let report = n.wheel_tracks(&input, LIMITS, TRACK_WORK_LIMIT);
        if let Some(wheel) = expected {
            assert!(matches!(report.outcome,TrackOutcome::Outside{wheel:w,at_nm:0,..} if w==wheel));
        } else {
            assert!(matches!(report.outcome, TrackOutcome::Covered(_)));
        }
        let outer = n.wheel_tracks(
            &TrackQuery {
                trace: RearAxleTrace::Line(RearAxleLine {
                    start_xz_m: [3.25, -2.],
                    ..trace
                }),
                ..input
            },
            LIMITS,
            TRACK_WORK_LIMIT,
        );
        assert!(
            matches!(outer.outcome, TrackOutcome::Outside {wheel:Wheel::FrontLeft,at_nm:0,overrun_lower_m:Some(d)} if (0.049..=0.050).contains(&d)),
            "{:?}",
            outer.outcome
        );
        let invalid = [AuthoringLaneOrdinal::try_from_usize(100).unwrap()];
        for wrong in [
            TrackQuery {
                frame: CanonicalFrameOrdinal::try_from_usize(1).unwrap(),
                ..input
            },
            TrackQuery {
                traversable_lanes: &invalid,
                ..input
            },
        ] {
            assert!(matches!(
                n.wheel_tracks(&wrong, LIMITS, TRACK_WORK_LIMIT).outcome,
                TrackOutcome::Unproven {
                    reason: Failure::Binding,
                    ..
                }
            ));
        }
    }
}

fn spiral(radius: f64, spiral: bool) -> Program {
    let mut segments = vec![Segment::line(p(0., 0., 0.))];
    if spiral {
        segments.push(
            Segment::clothoid(32., 0., 1. / 80., 0.)
                .unwrap()
                .with_smooth_entry(true),
        );
    }
    segments.push(
        Segment::circular_arc(16., radius, 0.)
            .unwrap()
            .with_smooth_entry(spiral),
    );
    Program::try_new(p(0., 0., -16.), segments).unwrap()
}

fn corridor_query(n: &SharedNetworkRevision, stations_nm: [u64; 2], offset: f64) -> TrackQuery<'_> {
    let mut input = query(
        n,
        RearAxleLine {
            start_xz_m: [0.; 2],
            heading_xz: [0., 1.],
            length_nm: 0,
        },
    );
    input.trace = RearAxleTrace::Corridor(CorridorTrace {
        stations_nm,
        from_m: [offset; 2],
        to_m: [offset; 2],
    });
    input
}

#[test]
fn spiral_checks_both_steering_rates_and_the_committed_join() {
    let lanes = [([8.; 2], true), ([3.5; 2], true)];
    let mut results = Vec::new();
    for radius in [80_f64, 80_f64.next_up()] {
        let artifact = road(spiral(radius, true), lanes);
        let n = load_network(&artifact, SpatialBuildOption::Omit);
        let mut reports = Vec::new();
        for stations in [[14, 50], [50, 14]] {
            let q = corridor_query(&n, stations.map(|v| v * 1_000_000_000), 0.);
            let slow = n.wheel_tracks(&q, LIMITS, TRACK_WORK_LIMIT);
            assert!(
                matches!(slow.outcome, TrackOutcome::Covered(_)),
                "{stations:?} {slow:?}"
            );
            let fast = n.wheel_tracks(
                &q,
                SteeringLimits {
                    speed_m_s: 20.,
                    ..LIMITS
                },
                TRACK_WORK_LIMIT,
            );
            assert!(
                matches!(
                    fast.outcome,
                    TrackOutcome::Infeasible {
                        constraint: SteeringConstraint::Rate,
                        ..
                    }
                ),
                "{fast:?}"
            );
            reports.push((slow, fast));
            let TrackOutcome::Infeasible { at_nm, wheel, .. } = fast.outcome else {
                unreachable!()
            };
            let direction = if stations[1] > stations[0] { 1. } else { -1. };
            let at = stations[0] as f64 + direction * (at_nm as f64 / 1e9);
            assert!((16.0..=48.0).contains(&at));
            let k = direction * (at - 16.) / 2560.;
            let b = if wheel == Wheel::FrontLeft { 0.8 } else { -0.8 };
            assert!(20. * 2.6 / 2560. / ((1. - b * k).powi(2) + (2.6 * k).powi(2)) > 0.020);
        }
        results.push((n.network_revision(), reports));
    }
    assert_eq!(results[0], results[1]);
    let artifact = road(spiral(80., false), lanes);
    let n = load_network(&artifact, SpatialBuildOption::Omit);
    let q = corridor_query(&n, [14_000_000_000, 18_000_000_000], 0.);
    let report = n.wheel_tracks(&q, LIMITS, TRACK_WORK_LIMIT);
    assert!(
        matches!(
            report.outcome,
            TrackOutcome::Infeasible {
                constraint: SteeringConstraint::Jump,
                at_nm: 2_000_000_000,
                ..
            }
        ),
        "{report:?}"
    );
}

#[test]
fn equal_published_revisions_produce_equal_poses_and_steering() {
    let line = |y| {
        Program::try_new(
            p(0., 0., 0.),
            vec![Segment::line(p(0., 1., 32.)), Segment::line(p(0., y, 64.))],
        )
        .unwrap()
    };
    let arc = |r| {
        Program::try_new(
            p(0., 0., -16.),
            vec![
                Segment::line(p(0., 0., 0.)),
                Segment::clothoid(32., 0., 1. / 64., 0.)
                    .unwrap()
                    .with_smooth_entry(true),
                Segment::circular_arc(16., r, 0.)
                    .unwrap()
                    .with_smooth_entry(true),
            ],
        )
        .unwrap()
    };
    for (curves, station) in [
        ([line(2.), line(2_f64.next_up())], 32_u64),
        ([arc(64.), arc(64_f64.next_up())], 48),
    ] {
        assert_ne!(curves[0], curves[1]);
        let networks = curves.map(|c| {
            load_network(
                &road(c, [([8.; 2], true), ([3.5; 2], true)]),
                SpatialBuildOption::Omit,
            )
        });
        assert_eq!(
            networks[0].network_revision(),
            networks[1].network_revision()
        );
        let reports = networks.each_ref().map(|n| {
            let q = corridor_query(
                n,
                [(station - 2) * 1_000_000_000, (station + 2) * 1_000_000_000],
                0.,
            );
            n.vehicle_geometry(
                &q,
                SteeringLimits {
                    speed_m_s: 5.,
                    ..LIMITS
                },
                BODY,
                BUDGET,
            )
        });
        assert!(reports[0].feasible(), "{:?}", reports[0]);
        assert_eq!(reports[0], reports[1]);
    }
}

const BODY: BodyEnvelope = BodyEnvelope {
    rear_overhang_m: 0.75,
    front_from_rear_m: 3.40,
    half_width_m: 0.95,
    heights_m: [0., 1.50],
};
const BUDGET: GeometryWorkLimits = GeometryWorkLimits::STANDARD;
fn extent(
    key: &str,
    low: BoundaryStation,
    high: BoundaryStation,
    payload: BoundaryPayloadInput,
) -> BoundaryExtentInput {
    let [left, right] = ["inner", "outer"].map(|key| {
        BoundaryMemberInput::AuthoringLane(
            AuthoringLaneReference::owner_scoped(vec!["part-0".into(), "section".into()], key)
                .unwrap(),
        )
    });
    BoundaryExtentInput {
        left,
        right,
        kind: if matches!(payload, BoundaryPayloadInput::CrossingRule { .. }) {
            BoundaryExtentKind::Rule
        } else {
            BoundaryExtentKind::Obstacle
        },
        local_key: key.into(),
        low,
        high,
        payload: Some(payload),
        canvas_selection: None,
    }
}
fn with_boundaries(
    curve: Program,
    width: f64,
    extents: Vec<BoundaryExtentInput>,
) -> PortablePublicationCandidate {
    let source = save_lane_declarations(
        curve.clone(),
        &[],
        (
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Smooth1Deg,
        ),
        [RoadEditingLaneDirection::Forward; 2],
        CrossSectionCoverage::Complete,
        0,
        SectionFixture {
            lanes: [([width; 2], true), ([3.5; 2], true)],
            extents,
            ..SectionFixture::default()
        },
    );
    assert_eq!(read(&source), curve);
    publish(&compile_saved(&source).unwrap(), "vehicle-boundaries")
}

// 圆弧上先在后轴截面的切线坐标里求真实双轴姿态，再旋转到护栏端帽坐标。
// 这个对照只使用来源半径、纵坡和给定车型。
fn arc_body(bias: f64, u: f64, [a, b, c]: [f64; 3]) -> [f64; 3] {
    let g = 40. / 399.;
    let rise = |d: f64, side: f64| g * 35. * d.atan2(35. + bias - side);
    let mut interval = [0., AXLES.wheelbase_m];
    for _ in 0..60 {
        let d = (interval[0] + interval[1]) * 0.5;
        let dy = (rise(d, 0.8) + rise(d, -0.8)) * 0.5;
        if d.hypot(dy) < AXLES.wheelbase_m {
            interval[0] = d;
        } else {
            interval[1] = d;
        }
    }
    let d = interval[0];
    let dy = (rise(d, 0.8) + rise(d, -0.8)) * 0.5;
    let t = [0., dy / 2.6, d / 2.6];
    let side = (rise(d, 0.8) - rise(d, -0.8)) * 0.5;
    let raw = [1.6, side * (1. - t[1] * t[1]), -side * t[1] * t[2]];
    let length = raw[0].hypot(raw[1]).hypot(raw[2]);
    let l = raw.map(|v| v / length);
    let n = [t[1] * l[2] - t[2] * l[1], t[2] * l[0], -t[1] * l[0]];
    let [x, y, z] = std::array::from_fn(|i| a * t[i] + b * l[i] + c * n[i]);
    let (sn, cs) = ((u - 12.) / 35.).sin_cos();
    let radial = 35. + bias - x;
    // x 沿端帽截面的径向向外，z 随站位增加。轴线为 z∈[-2,0]。
    [
        radial * cs - z * sn - (35. + 19. / 12.),
        (u - 12.) * g + 0.55 + y,
        radial * sn + z * cs,
    ]
}

#[test]
fn graded_guardrail_caps_and_vertical_clearance_share_one_pose() {
    let grade = 40. / 399.;
    let h = 401. / 399.;
    let curve = Program::try_new_with_tangent(
        p(0., 0., 0.),
        vec![Segment::circular_arc(24., 35., grade).unwrap()],
        Some([0., 1.]),
    )
    .unwrap();
    for (bias, height, error, expected) in [
        (0.20, 0., 0.0005, 1),
        (0.32, 0., 0.01, 0),
        (0.326, 0., 0.0005, 0),
        (0.46, 0., 0.0005, -1),
        (0.46, 3., 0.0005, 1),
    ] {
        let rail = extent(
            "rail",
            BoundaryStation::Fixed(10. * h),
            BoundaryStation::Fixed(12. * h),
            BoundaryPayloadInput::Guardrail {
                anchor_role: 2,
                lateral_offset_meters: 0.,
                horizontal_length_meters: 2.,
                radius_meters: 0.15,
                bottom_height_meters: 0.30 + height,
                top_height_meters: 0.85 + height,
                parameter_error_meters: error,
            },
        );
        let permission = extent(
            "mark",
            BoundaryStation::CorridorEndpoint,
            BoundaryStation::CorridorEndpoint,
            BoundaryPayloadInput::CrossingRule {
                directions: 3,
                effect: 1,
            },
        );
        let artifact = with_boundaries(curve.clone(), 19. / 6., vec![rail, permission]);
        let n = load_network(&artifact, SpatialBuildOption::Omit);
        let q = corridor_query(&n, [6., 14.].map(|x| (x * h * 1e9).round() as u64), -bias);
        let result = n.vehicle_geometry(&q, LIMITS, BODY, BUDGET);
        assert!(
            matches!(result.tracks.outcome, TrackOutcome::Covered(_)),
            "bias={bias} {:?}",
            result.tracks
        );
        match result.sweep.outcome {
            SweepOutcome::Clear(Some(clearance)) => {
                let (m, id) = (clearance.lower_m, clearance.boundary);
                assert_eq!(expected, 1, "{bias} {m}");
                assert!(m > 0.);
                assert_eq!(id.key, Some("rail"));
            }
            SweepOutcome::Contact {
                boundary,
                body_point_m,
                at_nm,
            } => {
                assert_eq!(expected, -1, "{bias} {body_point_m:?}");
                assert_eq!(boundary.key, Some("rail"));
                let u = 6. + at_nm as f64 / (h * 1e9);
                let [x, y, z] = arc_body(bias, u, body_point_m);
                assert!(
                    x.hypot(z - z.clamp(-2., 0.)) <= 0.15 && (0.30..=0.85).contains(&y),
                    "{u} {:?}",
                    [x, y, z]
                );
            }
            SweepOutcome::Unproven {
                boundary: Some(id),
                reason: Failure::Boundary,
                ..
            } if expected == 0 => {
                assert_eq!(id.key, Some("rail"));
                let [x, y, z] = arc_body(bias, 12., [3.4, -0.95, 0.]);
                let outer = (35. + 19. / 12. + x).hypot(z);
                let gap = 35. + 19. / 12. - outer - 0.15;
                assert!(
                    gap.abs() < error
                        && (0.30..=0.85).contains(
                            &(y - (40. / 399.) * 35. * (z / (35. + 19. / 12. + x)).atan())
                        )
                );
            }
            _ => panic!("bias {bias}, height {height}, {:?}", result.sweep),
        }
    }
}

#[test]
fn whole_body_checks_the_half_open_rule_in_the_actual_crossing_direction() {
    let straight = Program::try_new(p(0., 0., 0.), vec![Segment::line(p(0., 0., 70.))]).unwrap();
    let allow = extent(
        "upstream",
        BoundaryStation::CorridorEndpoint,
        BoundaryStation::Fixed(27.),
        BoundaryPayloadInput::CrossingRule {
            directions: 3,
            effect: 1,
        },
    );
    let mut downstream = allow.clone();
    downstream.local_key = "downstream".into();
    downstream.low = BoundaryStation::Fixed(27.);
    downstream.high = BoundaryStation::CorridorEndpoint;
    for (start, end, direction, denied) in [
        (27., 28., 1, true),
        (27., 28., 2, false),
        (48., 50., 1, false),
    ] {
        let ban = extent(
            "stripe",
            BoundaryStation::Fixed(start),
            BoundaryStation::Fixed(end),
            BoundaryPayloadInput::CrossingRule {
                directions: direction,
                effect: 2,
            },
        );
        let artifact = with_boundaries(
            straight.clone(),
            3.5,
            vec![ban, allow.clone(), downstream.clone()],
        );
        let n = load_network(&artifact, SpatialBuildOption::Omit);
        let mut q = corridor_query(&n, [12_000_000_000, 42_000_000_000], 0.);
        q.trace = RearAxleTrace::Corridor(CorridorTrace {
            stations_nm: [12_000_000_000, 42_000_000_000],
            from_m: [0.; 2],
            to_m: [-3.5; 2],
        });
        let report = n.vehicle_geometry(
            &q,
            SteeringLimits {
                speed_m_s: 1.,
                max_rate_rad_s: 2.,
                ..LIMITS
            },
            BODY,
            BUDGET,
        );
        assert!(
            matches!(report.tracks.outcome, TrackOutcome::Covered(_)),
            "{:?}",
            report.tracks
        );
        if denied {
            let SweepOutcome::Denied {
                boundary,
                at_nm,
                touch_station_m,
            } = report.sweep.outcome
            else {
                panic!("{:?}", report.sweep);
            };
            assert_eq!(boundary.key, Some("stripe"));
            assert!(
                touch_station_m[0] >= start && touch_station_m[1] < end,
                "{touch_station_m:?}"
            );
            assert!(
                12. + (at_nm as f64) * 1e-9 < start,
                "front touches before rear: {at_nm}"
            );
        } else {
            assert!(
                matches!(report.sweep.outcome, SweepOutcome::Clear(None)),
                "{:?}",
                report.sweep
            );
        }
    }
}

#[test]
fn independent_limits_preserve_answers_across_workers_and_spatial_builds() {
    let artifact = road(crest(), [([8.; 2], true), ([3.5; 2], true)]);
    let networks = [
        SpatialBuildOption::Omit,
        SpatialBuildOption::RetainAvailable,
    ]
    .map(|option| load_network(&artifact, option));
    let mut baseline = None;
    for n in &networks {
        let q = query(
            n,
            RearAxleLine {
                start_xz_m: [0., -3.],
                heading_xz: [0., 1.],
                length_nm: 6_000_000_000,
            },
        );
        let full = n.vehicle_geometry(&q, LIMITS, BODY, BUDGET);
        assert_eq!(*baseline.get_or_insert(full), full);
        assert!(full.feasible());
        assert!(
            matches!(full.tracks.outcome, TrackOutcome::Covered(_))
                && matches!(full.sweep.outcome, SweepOutcome::Clear(None)),
            "{full:?}"
        );
        let exact = [full.tracks.work, full.sweep.work];
        for budgets in [exact, exact.map(|i| i + 1), [u32::MAX; 2]] {
            assert_eq!(
                n.vehicle_geometry(
                    &q,
                    LIMITS,
                    BODY,
                    GeometryWorkLimits {
                        tracks: budgets[0],
                        sweep: budgets[1]
                    }
                ),
                full
            );
        }
        for (axis, other) in [(0, 1), (1, 0)] {
            let mut budgets = exact;
            budgets[axis] -= 1;
            let report = n.vehicle_geometry(
                &q,
                LIMITS,
                BODY,
                GeometryWorkLimits {
                    tracks: budgets[0],
                    sweep: budgets[1],
                },
            );
            let usage = [report.tracks.work, report.sweep.work];
            assert_eq!(usage[axis], exact[axis] - 1);
            assert_eq!(usage[other], exact[other]);
            if axis == 0 {
                assert!(report.tracks.unchecked_nm.is_some());
                assert_eq!(report.sweep, full.sweep);
            } else {
                assert!(report.sweep.unchecked_nm.is_some());
                assert_eq!(report.tracks, full.tracks);
            }
        }
        std::thread::scope(|scope| {
            let workers: Vec<_> = (0..4)
                .map(|_| scope.spawn(|| n.vehicle_geometry(&q, LIMITS, BODY, BUDGET)))
                .collect();
            for worker in workers {
                assert_eq!(worker.join().unwrap(), full);
            }
        });
    }
}

#[test]
fn curb_contact_uses_the_published_height_and_stable_local_key() {
    let curve = Program::try_new(p(0., 0., 0.), vec![Segment::line(p(0., 0., 70.))]).unwrap();
    let mark = extent(
        "open",
        BoundaryStation::CorridorEndpoint,
        BoundaryStation::CorridorEndpoint,
        BoundaryPayloadInput::CrossingRule {
            directions: 3,
            effect: 1,
        },
    );
    for (top, contact) in [(0.15, false), (0.75, true)] {
        let end = CurbEndInput {
            left_offset_meters: 0.10,
            right_offset_meters: -0.15,
            bottom_height_meters: 0.,
            top_height_meters: top,
        };
        let curb = |key| {
            extent(
                key,
                BoundaryStation::Fixed(25.),
                BoundaryStation::Fixed(40.),
                BoundaryPayloadInput::Curb {
                    low: end,
                    high: CurbEndInput {
                        left_offset_meters: 0.12,
                        ..end
                    },
                    parameter_error_meters: 1e-6,
                },
            )
        };
        let artifact = with_boundaries(
            curve.clone(),
            3.5,
            vec![curb("second"), mark.clone(), curb("first")],
        );
        let network = load_network(&artifact, SpatialBuildOption::Omit);
        let q = corridor_query(&network, [20_000_000_000, 35_000_000_000], -0.85);
        let r = network.vehicle_geometry(&q, LIMITS, BODY, BUDGET);
        assert!(matches!(r.tracks.outcome, TrackOutcome::Covered(_)));
        if contact {
            assert!(
                matches!(r.sweep.outcome,SweepOutcome::Contact{boundary,..} if boundary.key==Some("first")),
                "{:?}",
                r.sweep
            );
        } else {
            assert!(
                matches!(r.sweep.outcome,SweepOutcome::Clear(Some(gap)) if gap.lower_m>0.39),
                "{:?}",
                r.sweep
            );
        }
    }
}

#[test]
fn oblique_curb_clearance_is_proven_at_a_single_pose_and_in_motion() {
    let end = CurbEndInput {
        left_offset_meters: 0.10,
        right_offset_meters: -0.15,
        bottom_height_meters: 0.,
        top_height_meters: 0.15,
    };
    let curb = extent(
        "curb",
        BoundaryStation::Fixed(25.),
        BoundaryStation::Fixed(40.),
        BoundaryPayloadInput::Curb {
            low: end,
            high: end,
            parameter_error_meters: 1e-6,
        },
    );
    for (dx, dz) in [(1_f64, 1_f64), (0., 1.), (-1., 1.)] {
        let straight = Program::try_new(
            p(0., 0., 0.),
            vec![Segment::line(p(50. * dx, 0., 50. * dz))],
        )
        .unwrap();
        let artifact = with_boundaries(straight, 3.5, vec![curb.clone()]);
        let network = load_network(&artifact, SpatialBuildOption::Omit);
        let norm = dx.hypot(dz);
        let drive = |right_m: f64, length_nm| {
            let mut input = query(
                &network,
                RearAxleLine {
                    start_xz_m: [
                        (30. * dx - right_m * dz) / norm,
                        (30. * dz + right_m * dx) / norm,
                    ],
                    heading_xz: [dx, dz],
                    length_nm,
                },
            );
            input.axles.reference_height_m = 0.10;
            network.vehicle_geometry(&input, LIMITS, BODY, BUDGET)
        };
        for length_nm in [0, 4_000_000_000] {
            let report = drive(0., length_nm);
            let SweepOutcome::Clear(Some(gap)) = report.sweep.outcome else {
                panic!("heading {dx} {dz}, length {length_nm}: {:?}", report.sweep);
            };
            assert!(
                report.feasible()
                    && report.sweep.covered_prefix_nm == length_nm
                    && gap.boundary.key == Some("curb")
            );
            // 净空解析值为 1.75 - 0.10 - 0.95 = 0.70 米。运动区间可返回较宽的下界。
            assert!(gap.lower_m > 0. && gap.lower_m <= 0.70, "{gap:?}");
            if length_nm == 0 {
                assert!(gap.lower_m > 0.699, "{gap:?}");
            }
        }
        // 向路缘移动 70 厘米时净空落入参数误差，再多移 6 厘米有确定接触。
        let tangent = drive(0.70, 0).sweep;
        let SweepOutcome::Unproven {
            reason,
            boundary: Some(hit),
            ..
        } = tangent.outcome
        else {
            panic!("{dx} {dz}: {tangent:?}");
        };
        assert_eq!(reason, Failure::Boundary);
        assert_eq!(hit.key, Some("curb"));
        let contact = drive(0.76, 0).sweep;
        let SweepOutcome::Contact { boundary, .. } = contact.outcome else {
            panic!("{dx} {dz}: {contact:?}");
        };
        assert_eq!(boundary.key, Some("curb"));
    }
}

#[test]
fn short_lane_change_uses_the_motion_of_the_parts_touching_the_one_way_marking() {
    let straight = Program::try_new(p(0., 0., 0.), vec![Segment::line(p(0., 0., 50.))]).unwrap();
    let rule = |key, directions, effect| {
        extent(
            key,
            BoundaryStation::CorridorEndpoint,
            BoundaryStation::CorridorEndpoint,
            BoundaryPayloadInput::CrossingRule { directions, effect },
        )
    };
    let artifact = with_boundaries(
        straight,
        3.5,
        vec![rule("right", 1, 1), rule("no-left", 2, 2)],
    );
    let network = load_network(&artifact, SpatialBuildOption::Omit);
    let limits = SteeringLimits {
        max_angle_rad: 0.6,
        max_rate_rad_s: 0.6,
        speed_m_s: 1.,
    };
    let stations_nm = [12_000_000_000, 22_000_000_000];
    // 最后一条只右移 1 米。行程 7.5 米时，车头回正，距车身中线右侧约 0.50 米的部位向左触线。
    for (from, to, allowed) in [(0., -3.5, true), (-3.5, 0., false), (0., -1., false)] {
        let mut input = corridor_query(&network, stations_nm, from);
        input.trace = RearAxleTrace::Corridor(CorridorTrace {
            stations_nm,
            from_m: [from; 2],
            to_m: [to; 2],
        });
        let report = network.vehicle_geometry(&input, limits, BODY, BUDGET);
        if allowed {
            // 6.95 米处前左角在标线右侧约 1.75 米，正在向左回正；车尾仍向右跨线。
            assert!(
                matches!(report.sweep.outcome, SweepOutcome::Clear(None)),
                "{:?}",
                report.sweep
            );
            assert_eq!(report.sweep.covered_prefix_nm, 10_000_000_000);
        } else {
            let SweepOutcome::Denied { boundary, .. } = report.sweep.outcome else {
                panic!("{from} → {to}: {:?}", report.sweep);
            };
            assert_eq!(boundary.key, Some("no-left"));
        }
        assert!(
            matches!(report.tracks.outcome, TrackOutcome::Covered(_)),
            "{:?}",
            report.tracks
        );
    }
}

// §4.1 平路解析对照。后轴到前保险杠的投影使用车体切向；行程前半段的横坐标严格递增。
fn straight_front_handoff_nm(start_nm: u64) -> [u64; 2] {
    assert_eq!(BODY.front_from_rear_m, 3.40);
    let front = |x: f64| {
        let u = x / 30.;
        let y = 3.5 * u.powi(3) * (10. - 15. * u + 6. * u * u);
        let dy_dx = 3.5 * u.powi(2) * (1. - u).powi(2);
        let along = BODY.front_from_rear_m / dy_dx.hypot(1.);
        (x + along, y + along * dy_dx)
    };
    let (mut lo, mut hi) = (0., 15.);
    for _ in 0..36 {
        let mid = 0.5 * (lo + hi);
        if front(mid).1 < 1.75 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let (before, after) = (front(lo), front(hi));
    assert!(before.1 < 1.75 && after.1 >= 1.75);
    assert!((11.861_185..11.861_195).contains(&lo));
    assert!((15.195_155..15.195_165).contains(&before.0));
    // 两侧解均在止于 15 米的交接窗口外。此处仅校验参考截面及其静态地址。
    assert!(before.0 > 15. && after.0 >= before.0);
    [
        start_nm + (before.0 * 1e9).floor() as u64,
        start_nm + (after.0 * 1e9).ceil() as u64,
    ]
}

#[test]
fn handoff_checks_decreasing_station_and_lane_steering_at_both_endpoints() {
    use laneflow_static_network::vehicle::{HandoffOutcome, HandoffQuery, HandoffWindow};
    let straight = Program::try_new(p(0., 0., 0.), vec![Segment::line(p(0., 0., 50.))]).unwrap();
    let turn = Program::try_new(
        p(0., 0., 0.),
        vec![
            Segment::line(p(0., 0., 20.)),
            Segment::circular_arc(50., 64., 0.).unwrap(),
        ],
    )
    .unwrap();
    for (program, direction, rear, expected_seam) in [
        (
            straight,
            RoadEditingLaneDirection::Backward,
            [40, 10],
            false,
        ),
        (
            turn.clone(),
            RoadEditingLaneDirection::Forward,
            [20, 50],
            true,
        ),
        (turn, RoadEditingLaneDirection::Forward, [5, 20], true),
    ] {
        let mark = extent(
            "mark",
            BoundaryStation::CorridorEndpoint,
            BoundaryStation::CorridorEndpoint,
            BoundaryPayloadInput::CrossingRule {
                directions: 3,
                effect: 1,
            },
        );
        let source = save_lane_declarations(
            program,
            &[],
            (
                GeometryAccuracyProfile::Fine2Cm,
                GeometryDirectionProfile::Smooth1Deg,
            ),
            [direction; 2],
            CrossSectionCoverage::Complete,
            0,
            SectionFixture {
                extents: vec![mark],
                ..SectionFixture::default()
            },
        );
        let artifact = publish(&compile_saved(&source).unwrap(), "handoff-steering");
        let n = load_network(&artifact, SpatialBuildOption::Omit);
        let rear = rear.map(|m| m * 1_000_000_000u64);
        let q = corridor_query(&n, rear, 0.);
        let r = n.traffic().relations();
        let qh = HandoffQuery {
            revision: q.revision,
            corridor: q.corridor,
            frame: q.frame,
            axles: q.axles,
            rear_stations_nm: rear,
            station_window_nm: q.station_window_nm,
        };
        let laneflow_static_network::CorridorElement::RoadSection(section) =
            r.corridor_elements(q.corridor).unwrap()[0]
        else {
            unreachable!()
        };
        let lanes = r.section_lanes(section).unwrap();
        let window = HandoffWindow {
            edges: [0, 1].map(|i| r.authoring_edge_chain(lanes[i]).unwrap()[0]),
            stations_nm: q.station_window_nm,
            left_index: 0,
            owner: laneflow_static_network::BoundaryOwner::Section(section),
        };
        let handoff = n.vehicle_handoff(
            &qh,
            SteeringLimits {
                max_angle_rad: 0.6,
                max_rate_rad_s: 0.6,
                speed_m_s: 1.,
            },
            BODY,
            BUDGET,
            window,
        );
        if expected_seam {
            assert!(
                matches!(
                    handoff.outcome,
                    HandoffOutcome::Unproven {
                        reason: Failure::SteeringSeam,
                        ..
                    }
                ),
                "{handoff:?}"
            );
        } else {
            let HandoffOutcome::Located { stations_nm, .. } = handoff.outcome else {
                panic!("{handoff:?}");
            };
            let [a, b] = straight_front_handoff_nm(0);
            assert!(stations_nm[0] <= rear[0] - b && stations_nm[1] >= rear[0] - a);
        }
    }
}

#[test]
fn lane_change_geometry_crosses_independent_edge_cuts() {
    use laneflow_static_network::StationQuery;

    // 横移前后各留 10 米路面，横移起点两侧均开新边，使局部进度可与 §4.1 直接对照。
    let straight = Program::try_new(p(0., 0., -10.), vec![Segment::line(p(0., 0., 40.))]).unwrap();
    let stations_nm = [10_000_000_000, 40_000_000_000];
    let handoff_nm = straight_front_handoff_nm(stations_nm[0]);
    let limits = SteeringLimits {
        max_angle_rad: 0.75_f64.atan(),
        max_rate_rad_s: 0.6,
        speed_m_s: 1.,
    };
    for effect in [1, 2] {
        let marking = extent(
            "mark",
            BoundaryStation::CorridorEndpoint,
            BoundaryStation::CorridorEndpoint,
            BoundaryPayloadInput::CrossingRule {
                directions: 3,
                effect,
            },
        );
        let source = save_lane_declarations(
            straight.clone(),
            &[],
            (
                GeometryAccuracyProfile::Fine2Cm,
                GeometryDirectionProfile::Smooth1Deg,
            ),
            [RoadEditingLaneDirection::Forward; 2],
            CrossSectionCoverage::Complete,
            0,
            SectionFixture {
                lane_cuts_m: [&[10., 22.], &[10., 28.]],
                extents: vec![marking],
                ..SectionFixture::default()
            },
        );
        let artifact = publish(&compile_saved(&source).unwrap(), "independent-lane-change");
        let networks = [
            SpatialBuildOption::Omit,
            SpatialBuildOption::RetainAvailable,
        ]
        .map(|spatial| load_network(&artifact, spatial));
        let reports = networks.each_ref().map(|network| {
            let mut q = corridor_query(network, stations_nm, 0.);
            q.trace = RearAxleTrace::Corridor(CorridorTrace {
                stations_nm,
                from_m: [0.; 2],
                to_m: [-3.5; 2],
            });
            // 将同一车头截面和切点附近的占用区间分别投影到源、目标两条独立边链。
            for (&lane, (index, progress_mm)) in
                q.traversable_lanes.iter().zip([(2, 3_195), (1, 15_195)])
            {
                let chain = network
                    .traffic()
                    .relations()
                    .authoring_edge_chain(lane)
                    .unwrap();
                assert_eq!(chain.len(), 3);
                let addresses = |range| {
                    let at = network
                        .station_mapping()
                        .in_station_range(q.corridor, range, lane);
                    let StationQuery::Complete(hits) = at else {
                        panic!("{at:?}")
                    };
                    assert_eq!(hits.stations_nm(), range);
                    hits.addresses().collect::<Vec<_>>()
                };
                let hit = addresses(handoff_nm);
                assert_eq!(hit.len(), 1);
                assert_eq!(hit[0].edge, chain[index]);
                assert_eq!(hit[0].progress_mm, [progress_mm; 2]);
                for (cut, expected) in [
                    (
                        22_000_000_000,
                        if index == 2 {
                            &chain[1..=2]
                        } else {
                            &chain[1..=1]
                        },
                    ),
                    (
                        28_000_000_000,
                        if index == 2 {
                            &chain[2..=2]
                        } else {
                            &chain[1..=2]
                        },
                    ),
                ] {
                    let hits = addresses([cut - 1_000_000, cut + 1_000_000]);
                    assert!(
                        hits.iter().map(|a| a.edge).eq(expected.iter().copied()),
                        "{hits:?}"
                    );
                }
            }
            use laneflow_static_network::vehicle::{HandoffOutcome, HandoffQuery, HandoffWindow};
            let r = network.traffic().relations();
            let handoff = HandoffQuery {
                revision: q.revision,
                corridor: q.corridor,
                frame: q.frame,
                station_window_nm: q.station_window_nm,
                axles: q.axles,
                rear_stations_nm: stations_nm,
            };
            let edges = [
                r.authoring_edge_chain(q.traversable_lanes[0]).unwrap()[2],
                r.authoring_edge_chain(q.traversable_lanes[1]).unwrap()[1],
            ];
            let window = HandoffWindow {
                edges,
                owner: laneflow_static_network::BoundaryOwner::Section(
                    r.authoring_section(q.traversable_lanes[0]).unwrap(),
                ),
                left_index: 0,
                stations_nm: [22_000_000_000, 26_000_000_000],
            };
            let wide = network.vehicle_handoff(&handoff, limits, BODY, BUDGET, window);
            if effect == 1 {
                let HandoffOutcome::Located {
                    stations_nm: front_nm,
                    trace_range_nm,
                    addresses,
                } = wide.outcome
                else {
                    panic!("{wide:?}");
                };
                assert!(
                    front_nm[0] <= handoff_nm[0] && front_nm[1] >= handoff_nm[1],
                    "{wide:?} {handoff_nm:?}"
                );
                assert!(trace_range_nm[0] < 11_861_195_000 && trace_range_nm[1] > 11_861_185_000);
                assert_eq!(addresses.map(|a| a.edge), edges);
                assert_eq!(addresses.map(|a| a.progress_mm), [[3_195; 2], [15_195; 2]]);
                let reversed = HandoffWindow { edges: [edges[1], edges[0]], ..window };
                assert!(matches!(
                    network.vehicle_handoff(&handoff, limits, BODY, BUDGET, reversed).outcome,
                    HandoffOutcome::Located { addresses: [a,b], .. } if [a.edge,b.edge] == reversed.edges
                ));
                let narrow = HandoffWindow {
                    stations_nm: [window.stations_nm[0], 25_000_000_000],
                    ..window
                };
                assert_eq!(
                    network
                        .vehicle_handoff(&handoff, limits, BODY, BUDGET, narrow)
                        .outcome,
                    HandoffOutcome::OutsideWindow {
                        stations_nm: front_nm
                    }
                );
                // 误差包络仅与窗口相交，或交接在该车道的另一次切边上，均不能定位到声明的出现对。
                assert!(front_nm[1] > front_nm[0]);
                let partial = HandoffWindow { stations_nm: [front_nm[1]; 2], ..window };
                let wrong_edge = HandoffWindow {
                    edges: [r.authoring_edge_chain(q.traversable_lanes[0]).unwrap()[1], edges[1]],
                    ..window
                };
                for (input, reason) in [(partial, Failure::Boundary), (wrong_edge, Failure::Binding)] {
                    assert_eq!(
                        network.vehicle_handoff(&handoff, limits, BODY, BUDGET, input).outcome,
                        HandoffOutcome::Unproven { range_nm: trace_range_nm, reason }
                    );
                }
                let wrong_seam = HandoffWindow { left_index: 1, ..window };
                let wrong = network.vehicle_handoff(&handoff, limits, BODY, BUDGET, wrong_seam);
                assert!(wrong.geometry.is_none());
                assert!(matches!(wrong.outcome, HandoffOutcome::Unproven {reason: Failure::Binding, ..}));
                let narrow_body = BodyEnvelope { half_width_m: AXLES.half_track_m - 0.01, ..BODY };
                let invalid = network.vehicle_handoff(&handoff, limits, narrow_body, BUDGET, window);
                assert!(invalid.geometry.is_none());
                assert!(matches!(invalid.outcome, HandoffOutcome::Unproven { reason: Failure::InvalidInput, .. }));
                let used = wide.work + wide.geometry.unwrap().sweep.work;
                for remaining in [used - 1, used] {
                    let limited = network.vehicle_handoff(
                        &handoff,
                        limits,
                        BODY,
                        GeometryWorkLimits {
                            sweep: remaining,
                            ..BUDGET
                        },
                        window,
                    );
                    assert_eq!(
                        limited.work + limited.geometry.unwrap().sweep.work,
                        remaining
                    );
                    if remaining == used {
                        assert_eq!(limited, wide);
                    } else {
                        assert!(matches!(
                            limited.outcome,
                            HandoffOutcome::Unproven {
                                reason: Failure::WorkLimit,
                                ..
                            }
                        ));
                    }
                }
            } else {
                assert_eq!(wide.outcome, HandoffOutcome::GeometryRejected);
            }
            wide
        });
        assert_eq!(reports[0], reports[1]);
        let r = reports[0].geometry.unwrap();
        assert!(
            matches!(r.tracks.outcome, TrackOutcome::Covered(_)),
            "{r:?}"
        );
        assert_eq!(r.tracks.covered_prefix_nm, 30_000_000_000);
        assert_eq!(r.feasible(), effect == 1, "{r:?}");
        if effect == 1 {
            assert_eq!(r.sweep.covered_prefix_nm, 30_000_000_000);
        } else {
            assert!(
                matches!(r.sweep.outcome, SweepOutcome::Denied { boundary, .. } if boundary.key == Some("mark")),
                "{r:?}"
            );
        }
    }
}
