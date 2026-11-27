use super::*;
use laneflow_static_network::{
    BoundaryOwner,
    vehicle::{HandoffBodySide, HandoffLaneSide, HandoffQuery, HandoffWindow, SWEEP_WORK_LIMIT},
};

const M: u64 = 1_000_000_000;

#[test]
fn curved_seam_can_cut_the_body_even_with_every_corner_on_the_target_side() {
    let radius: f64 = 64.;
    let mut segments = vec![Segment::line(p(0., 0., 20.))];
    segments.extend((0..5).map(|_| Segment::circular_arc(10., radius, 0.).unwrap()));
    let circle = Program::try_new(p(0., 0., 0.), segments).unwrap();
    let artifact = road(circle, [([3.5; 2], true); 2]);
    let n = load_network(&artifact, SpatialBuildOption::Omit);
    let start = 45 * M + 123_000_000;
    let (q, w) = inputs(&n, [start, start + 18 * M]);
    let wide = BodyEnvelope {
        front_from_rear_m: 20.,
        rear_overhang_m: 20.,
        ..BODY
    };
    // 起始后轴在半径 64 m 参考车道的中心，车体沿圆的切线。共边在半径
    // 65.75 m 处。前、后端的最内角都在共边外，但车身纵向中截面全在共边内。
    let seam_radius = radius + 1.75;
    assert!((radius - wide.half_width_m).hypot(wide.front_from_rear_m) > seam_radius);
    assert!(radius + wide.half_width_m < seam_radius);
    let r = n.handoff_body_side(&q, w, wide, [start; 2], SWEEP_WORK_LIMIT);
    assert_eq!(r.side, Ok(HandoffBodySide::Straddling), "{r:?}");
    let clipped = HandoffQuery {
        station_window_nm: [40 * M, 65 * M],
        ..q
    };
    assert_eq!(
        n.handoff_body_side(&clipped, w, wide, [start; 2], SWEEP_WORK_LIMIT)
            .side,
        Err(Failure::Projection)
    );
}

fn inputs(n: &SharedNetworkRevision, rear: [u64; 2]) -> (HandoffQuery, HandoffWindow) {
    let q = corridor_query(n, rear, 0.);
    let r = n.traffic().relations();
    let laneflow_static_network::CorridorElement::RoadSection(section) =
        r.corridor_elements(q.corridor).unwrap()[0]
    else {
        unreachable!()
    };
    let lanes = r.section_lanes(section).unwrap();
    let window = HandoffWindow {
        edges: [0, 1].map(|i| r.authoring_edge_chain(lanes[i]).unwrap()[0]),
        owner: BoundaryOwner::Section(section),
        left_index: 0,
        stations_nm: q.station_window_nm,
    };
    (
        HandoffQuery {
            revision: q.revision,
            corridor: q.corridor,
            frame: q.frame,
            station_window_nm: q.station_window_nm,
            axles: AXLES,
            rear_stations_nm: rear,
        },
        window,
    )
}

#[test]
fn body_side_at_handoff_endpoints_is_invariant_under_horizontal_rotation() {
    let diagonal = std::f64::consts::FRAC_1_SQRT_2;
    for heading in [[0., 1.], [diagonal, diagonal]] {
        for (direction, rear) in [
            (RoadEditingLaneDirection::Forward, [10 * M, 40 * M]),
            (RoadEditingLaneDirection::Backward, [40 * M, 10 * M]),
        ] {
            let artifact = straight_road(heading, direction, SectionFixture::default());
            let network = load_network(&artifact, SpatialBuildOption::Omit);
            let (q, window) = inputs(&network, rear);
            for edges in [window.edges, [window.edges[1], window.edges[0]]] {
                let w = HandoffWindow { edges, ..window };
                // 两端五次横移的切向都沿道路。两车道各宽 3.5 m，
                // 半车宽 0.95 m，起止姿态的完整车身距共边均有 0.80 m。
                for (station, expected) in rear
                    .into_iter()
                    .zip([HandoffBodySide::SourceOnly, HandoffBodySide::TargetOnly])
                {
                    let query = |limit| network.handoff_body_side(&q, w, BODY, [station; 2], limit);
                    let full = query(SWEEP_WORK_LIMIT);
                    assert_eq!(
                        full.side,
                        Ok(expected),
                        "{heading:?}, {direction:?}, {edges:?}"
                    );
                    assert_eq!(query(full.work), full);
                    assert_eq!(query(full.work - 1).side, Err(Failure::WorkLimit));
                }
            }
        }
    }
}

fn straight_road(
    [x, z]: [f64; 2],
    direction: RoadEditingLaneDirection,
    section: SectionFixture<'_>,
) -> PortablePublicationCandidate {
    let at = |s| p(x * s, 0., z * s);
    let program =
        Program::try_new(at(0.), vec![Segment::line(at(25.)), Segment::line(at(50.))]).unwrap();
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
        section,
    );
    publish(&compile_saved(&source).unwrap(), "handoff-body-side")
}

// 以行驶方向为纵轴、朝目标车道为横轴。平直路面上裁剪真实矩形和共边，
// 解析比较点不依赖生产路面投影或侧别判据。
fn flat_footprint(start: f64, sign: f64, advance: f64) -> (f64, [Option<[f64; 2]>; 2]) {
    let u = advance / 30.;
    let lateral = 3.5 * u.powi(3) * (10. + u * (-15. + 6. * u));
    let slope = 3.5 * u.powi(2) * (1. - u).powi(2);
    let cos = 1. / slope.hypot(1.);
    let point = |a: f64, b: f64| {
        [
            start + sign * (advance + cos * (a - b * slope)),
            lateral + cos * (a * slope + b) - 1.75,
        ]
    };
    let rear = -BODY.rear_overhang_m;
    let front = BODY.front_from_rear_m;
    let w = BODY.half_width_m;
    let corners = [
        point(rear, -w),
        point(rear, w),
        point(front, w),
        point(front, -w),
    ];
    let spans = [-1., 1.].map(|side| {
        let mut values = Vec::new();
        for (a, b) in corners.into_iter().zip(corners.into_iter().cycle().skip(1)) {
            if a[1] * side > 0. {
                values.push(a[0]);
            }
            if a[1] * b[1] < 0. {
                values.push(a[0] + (b[0] - a[0]) * a[1] / (a[1] - b[1]));
            }
        }
        if values.is_empty() {
            return None;
        }
        Some([
            values.iter().copied().fold(f64::INFINITY, f64::min),
            values.into_iter().fold(f64::NEG_INFINITY, f64::max),
        ])
    });
    (point(front, 0.)[0], spans)
}

#[test]
fn handoff_footprint_projects_the_front_and_witnessed_body_to_independently_cut_lanes() {
    for heading in [[0., 1.], [std::f64::consts::FRAC_1_SQRT_2; 2]] {
        for (direction, rear, sign) in [
            (RoadEditingLaneDirection::Forward, [10 * M, 40 * M], 1.),
            (RoadEditingLaneDirection::Backward, [40 * M, 10 * M], -1.),
        ] {
            let mut cuts = [
                [13.4, 22.5, 23.5, 25., 26.5, 27.5, 36.6],
                [13.4, 22., 23., 24., 26., 27., 36.6],
            ];
            if sign < 0. {
                cuts.iter_mut().for_each(|c| c.reverse());
            }
            let artifact = straight_road(
                heading,
                direction,
                SectionFixture {
                    lane_cuts_m: [&cuts[0], &cuts[1]],
                    ..SectionFixture::default()
                },
            );
            let n = load_network(&artifact, SpatialBuildOption::Omit);
            let (q, w) = inputs(&n, rear);
            let BoundaryOwner::Section(section) = w.owner else {
                unreachable!()
            };
            let lanes = n.traffic().relations().section_lanes(section).unwrap();
            let mapping = n.station_mapping();
            let start = rear[0] as f64 / M as f64;
            let crossing_nm = ((start + sign * 15.19516) * M as f64) as u64;
            for pair in [[lanes[0], lanes[1]], [lanes[1], lanes[0]]] {
                let edges = pair.map(|lane| {
                    let laneflow_static_network::StationQuery::Complete(hits) =
                        mapping.at_station(q.corridor, crossing_nm, lane)
                    else {
                        panic!("cut fixture")
                    };
                    hits.addresses().next().unwrap().edge
                });
                let window = HandoffWindow {
                    edges,
                    stations_nm: [crossing_nm - M / 10, crossing_nm + M / 10],
                    ..w
                };
                for (advance, expected) in [
                    (0, HandoffLaneSide::Source),
                    (13, HandoffLaneSide::Target),
                    (20, HandoffLaneSide::Target),
                    (30, HandoffLaneSide::Target),
                ] {
                    let rear_nm = if sign > 0. {
                        rear[0] + advance * M
                    } else {
                        rear[0] - advance * M
                    };
                    let query = |limit| n.handoff_footprint(&q, window, BODY, [rear_nm; 2], limit);
                    let report = query(SWEEP_WORK_LIMIT);
                    let footprint = report.footprint.as_ref().unwrap_or_else(|e| {
                        panic!("{heading:?}, {direction:?}, {edges:?}, {advance}, {e:?}")
                    });
                    assert_eq!(footprint.front_side, expected);
                    let (front, actual) = flat_footprint(start, sign, advance as f64);
                    let in_m = |s: [u64; 2]| s.map(|n| n as f64 / M as f64);
                    enclosed(in_m(footprint.front.stations_nm()), front);
                    if advance == 0 {
                        // 前杠位于独立边链的切点，几何误差横跨切点，不可选定唯一边。
                        assert_eq!(footprint.front.addresses().count(), 2);
                    }
                    let laneflow_static_network::StationQuery::Complete(front_at) = mapping
                        .at_station(
                            q.corridor,
                            (front * M as f64).round() as u64,
                            pair[expected as usize],
                        )
                    else {
                        panic!("front mapping")
                    };
                    assert!(
                        front_at
                            .addresses()
                            .all(|actual| footprint
                                .front
                                .addresses()
                                .any(|outer| outer.edge == actual.edge
                                    && outer.progress_mm[0] <= actual.progress_mm[0]
                                    && actual.progress_mm[1] <= outer.progress_mm[1]))
                    );
                    for (i, (side, span)) in footprint.sides.iter().zip(actual).enumerate() {
                        assert_eq!(side.is_some(), span.is_some());
                        if let (Some(side), Some([lo, hi])) = (side, span) {
                            let enclosure = in_m(side.enclosure.stations_nm());
                            enclosed(enclosure, lo);
                            enclosed(enclosure, hi);
                            let [a, b] = in_m(side.witness.stations_nm());
                            assert!(a <= hi && lo <= b, "witness {a}..{b} outside {lo}..{hi}");
                            let chain = n
                                .traffic()
                                .relations()
                                .authoring_edge_chain(pair[i])
                                .unwrap();
                            assert!(side.enclosure.addresses().all(|p| chain.contains(&p.edge)));
                            if advance == 13 {
                                assert!(side.enclosure.addresses().count() >= 3);
                            }
                            if advance == 20 && i == 0 {
                                // 此时只剩靠源侧的车尾角，不能把整车长度当作该侧端点。
                                assert!(enclosure[1] - enclosure[0] < 2.);
                            }
                        }
                    }
                    assert_eq!(query(report.work), report);
                    assert_eq!(query(report.work - 1).footprint, Err(Failure::WorkLimit));
                }
                let at = |offset: u64| {
                    if sign > 0. {
                        rear[0] + offset * M
                    } else {
                        rear[0] - offset * M
                    }
                };
                // 距起点 5 m 时，靠目标侧的前车角恰好触到共边，另一外缘仍有余量。
                // 共边接触与严格留在源侧需要不同证据。
                let u = 5.0_f64 / 30.;
                let y = 3.5 * u.powi(3) * (10. + u * (-15. + 6. * u));
                let slope = 3.5 * u.powi(2) * (1. - u).powi(2);
                let touching = BodyEnvelope {
                    half_width_m: (1.75 - y) * slope.hypot(1.) - BODY.front_from_rear_m * slope,
                    ..BODY
                };
                let s = at(5);
                for positions in [[s; 2], [s - 1_000_000, s + 1_000_000]] {
                    assert_eq!(
                        n.handoff_footprint(&q, window, touching, positions, SWEEP_WORK_LIMIT)
                            .footprint,
                        Err(Failure::Boundary)
                    );
                    let swept = n.handoff_swept_occupancy(
                        &q,
                        window,
                        touching,
                        positions,
                        SWEEP_WORK_LIMIT,
                    );
                    assert!(swept.sides.unwrap().iter().all(Option::is_some));
                }
                let a = at(11);
                let b = at(12);
                let middle = at(15);
                let both = n
                    .handoff_footprint(
                        &q,
                        window,
                        BODY,
                        [middle - 1_000_000, middle + 1_000_000],
                        SWEEP_WORK_LIMIT,
                    )
                    .footprint
                    .unwrap();
                assert_eq!(both.front_side, HandoffLaneSide::Target);
                assert!(both.sides.iter().all(Option::is_some));
                // 前杠穿越共边的区间，不能用一个已提交路线游标代表。
                assert!(
                    n.handoff_footprint(&q, window, BODY, [a.min(b), a.max(b)], SWEEP_WORK_LIMIT)
                        .footprint
                        .is_err()
                );
                let outside = BodyEnvelope {
                    half_width_m: 2.,
                    ..BODY
                };
                assert_eq!(
                    n.handoff_footprint(&q, window, outside, [rear[0]; 2], SWEEP_WORK_LIMIT)
                        .footprint,
                    Err(Failure::Boundary)
                );
                assert_eq!(
                    n.handoff_swept_occupancy(&q, window, outside, [rear[0]; 2], SWEEP_WORK_LIMIT)
                        .sides,
                    Err(Failure::Boundary)
                );
            }
        }
    }
}

#[test]
fn continuous_handoff_sweep_covers_intermediate_motion_across_independent_cuts() {
    for heading in [[0., 1.], [std::f64::consts::FRAC_1_SQRT_2; 2]] {
        for (direction, rear, sign) in [
            (RoadEditingLaneDirection::Forward, [10 * M, 40 * M], 1.),
            (RoadEditingLaneDirection::Backward, [40 * M, 10 * M], -1.),
        ] {
            let mut cuts = [[15., 23., 26., 35.], [14., 22., 28., 36.]];
            if sign < 0. {
                cuts.iter_mut().for_each(|c| c.reverse());
            }
            let artifact = straight_road(
                heading,
                direction,
                SectionFixture {
                    lane_cuts_m: [&cuts[0], &cuts[1]],
                    ..SectionFixture::default()
                },
            );
            let n = load_network(&artifact, SpatialBuildOption::Omit);
            let (q, w) = inputs(&n, rear);
            let BoundaryOwner::Section(section) = w.owner else {
                unreachable!()
            };
            let lanes = n.traffic().relations().section_lanes(section).unwrap();
            let start = rear[0] as f64 / M as f64;
            let at = |advance: f64| ((start + sign * advance) * M as f64).round() as u64;
            let mapping = n.station_mapping();
            for pair in [[lanes[0], lanes[1]], [lanes[1], lanes[0]]] {
                let edges = pair.map(|lane| {
                    let laneflow_static_network::StationQuery::Complete(hits) =
                        mapping.at_station(q.corridor, at(15.19516), lane)
                    else {
                        panic!("crossing")
                    };
                    hits.addresses().next().unwrap().edge
                });
                let w = HandoffWindow { edges, ..w };
                let sweep = |a: f64, b: f64, budget| {
                    let lo = at(a).min(at(b));
                    let hi = at(a).max(at(b));
                    n.handoff_swept_occupancy(&q, w, BODY, [lo, hi], budget)
                };
                for (a, b, present) in [(0., 1., [true, false]), (25., 26., [false, true])] {
                    let sides = sweep(a, b, SWEEP_WORK_LIMIT).sides.unwrap();
                    assert_eq!(sides.each_ref().map(Option::is_some), present);
                    let [lo, hi] = sides.iter().flatten().next().unwrap().stations_nm();
                    // 后轴走 1 m，4.15 m 车体在同一侧，外包局限在邻近 7 m 以内。
                    assert!(hi - lo < 7 * M, "{lo}..{hi}");
                }
                // 拍初仅在源侧，拍末已清空源侧。以独立的平面矩形解析投影检查两侧
                // 的中间切边与边内毫米；起终两个姿态的占用都没有覆盖这些范围。
                let full = sweep(0., 23., SWEEP_WORK_LIMIT);
                let sides = full
                    .sides
                    .as_ref()
                    .unwrap_or_else(|e| panic!("{heading:?} {direction:?} {e:?} {}", full.work));
                for (i, side) in sides.iter().enumerate() {
                    let span = side.as_ref().unwrap();
                    let bounds = span.stations_nm().map(|s| s as f64 / M as f64);
                    assert!(bounds[0] > 4. && bounds[1] < 46., "{bounds:?}");
                    assert!(span.addresses().count() >= 2);
                    for step in 0..=92 {
                        let (_, spans) = flat_footprint(start, sign, f64::from(step) / 4.);
                        if let Some(actual) = spans[i] {
                            for s in actual {
                                enclosed(bounds, s);
                                let laneflow_static_network::StationQuery::Complete(hits) = mapping
                                    .at_station(q.corridor, (s * M as f64).round() as u64, pair[i])
                                else {
                                    panic!("oracle mapping")
                                };
                                assert!(hits.addresses().all(|p| {
                                    span.addresses().any(|a| {
                                        a.edge == p.edge
                                            && a.progress_mm[0] <= p.progress_mm[0]
                                            && a.progress_mm[1] >= p.progress_mm[1]
                                    })
                                }));
                            }
                        }
                    }
                }
                assert_eq!(sweep(0., 23., full.work), full);
                let short = sweep(0., 23., full.work - 1);
                assert_eq!(
                    (short.sides, short.work),
                    (Err(Failure::WorkLimit), full.work - 1)
                );
            }
        }
    }
}

#[test]
fn swept_ranges_require_complete_edge_mapping_on_both_sides() {
    let dense: Vec<f64> = (19..=34).map(f64::from).collect();
    let artifact = straight_road(
        [0., 1.],
        RoadEditingLaneDirection::Forward,
        SectionFixture {
            lane_cuts_m: [&[], &dense],
            ..SectionFixture::default()
        },
    );
    let n = load_network(&artifact, SpatialBuildOption::Omit);
    let (q, w) = inputs(&n, [10 * M, 40 * M]);
    let sweep = |range| n.handoff_swept_occupancy(&q, w, BODY, range, SWEEP_WORK_LIMIT);
    // 源侧能单独映射，短段目标侧也能映射；合并目标侧整段将越过八条边。
    assert!(sweep([10 * M, 11 * M]).sides.unwrap()[0].is_some());
    assert!(sweep([32 * M, 33 * M]).sides.unwrap()[1].is_some());
    assert_eq!(sweep([10 * M, 33 * M]).sides, Err(Failure::Projection));
    assert_eq!(sweep([33 * M, 10 * M]).sides, Err(Failure::InvalidInput));
}

#[test]
fn full_body_straddles_after_front_handoff_and_then_clears_in_both_directions() {
    for (direction, rear) in [
        (RoadEditingLaneDirection::Forward, [10 * M, 40 * M]),
        (RoadEditingLaneDirection::Backward, [40 * M, 10 * M]),
    ] {
        let artifact = straight_road([0., 1.], direction, SectionFixture::default());
        let reports = [
            SpatialBuildOption::Omit,
            SpatialBuildOption::RetainAvailable,
        ]
        .map(|option| {
            let n = load_network(&artifact, option);
            let (q, window) = inputs(&n, rear);
            let station = |offset| {
                if rear[0] < rear[1] {
                    rear[0] + offset
                } else {
                    rear[0] - offset
                }
            };
            let mut reports = Vec::new();
            for edges in [window.edges, [window.edges[1], window.edges[0]]] {
                let w = HandoffWindow { edges, ..window };
                // 30 m 五次横移的前保险杠已在后轴前进 11.8612 m 时过缝。13 m
                // 时两侧仍各有车身；25 m 附近整个车身已在目标侧。
                for (offset, side) in [
                    (5 * M, HandoffBodySide::SourceOnly),
                    (13 * M, HandoffBodySide::Straddling),
                    (15 * M, HandoffBodySide::Straddling),
                    (25 * M, HandoffBodySide::TargetOnly),
                ] {
                    let s = station(offset);
                    let range = [s - 1_000_000, s + 1_000_000];
                    let full = n.handoff_body_side(&q, w, BODY, range, SWEEP_WORK_LIMIT);
                    assert_eq!(full.side, Ok(side), "{offset} {full:?}");
                    assert_eq!(n.handoff_body_side(&q, w, BODY, range, full.work), full);
                    assert_eq!(
                        n.handoff_body_side(&q, w, BODY, range, full.work - 1).side,
                        Err(Failure::WorkLimit)
                    );
                    reports.push(full);
                }
                let tangent = BodyEnvelope {
                    half_width_m: 1.75,
                    ..BODY
                };
                assert_eq!(
                    n.handoff_body_side(&q, w, tangent, [rear[0]; 2], SWEEP_WORK_LIMIT)
                        .side,
                    Err(Failure::Boundary)
                );
                assert!(
                    n.handoff_body_side(&q, w, BODY, [12 * M, 38 * M], SWEEP_WORK_LIMIT)
                        .side
                        .is_err()
                );
                for range in [[20 * M, 19 * M], [0; 2]] {
                    assert_eq!(
                        n.handoff_body_side(&q, w, BODY, range, SWEEP_WORK_LIMIT)
                            .side,
                        Err(Failure::InvalidInput)
                    );
                }
            }
            reports
        });
        assert_eq!(reports[0], reports[1]);
    }
}
